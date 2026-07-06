//! HTTP range-backed remote GeoTIFF/COG access.
//!
//! This module opens remote objects through the same TIFF decoder core used for
//! local files by providing a random-access byte source backed by cached range
//! requests.

use std::io::Read;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use tiff_reader::source::{SharedSource, TiffSource};
use tiff_reader::{OpenOptions as TiffOpenOptions, TiffFile};

use crate::http_range::{probe_total_from_content_range, validate_content_range_header};
use crate::{Error, GeoTiffFile, Result};

/// Options for HTTP range-backed GeoTIFF access.
#[derive(Debug, Clone)]
pub struct HttpOpenOptions {
    /// Fixed byte-range chunk size.
    pub chunk_size: usize,
    /// Maximum bytes retained in the range cache.
    pub cache_bytes: usize,
    /// Maximum cached chunks.
    pub cache_slots: usize,
    /// TCP connect timeout for clients built from these options.
    ///
    /// Ignored when `client` is provided; configure custom clients directly.
    pub connect_timeout: Option<Duration>,
    /// Timeout for each range response body read.
    pub read_timeout: Option<Duration>,
    /// Overall timeout applied to each HEAD or GET request.
    pub request_timeout: Option<Duration>,
    /// Headers sent on every HEAD and byte-range GET request.
    pub headers: HeaderMap,
    /// Optional preconfigured blocking client for custom TLS, proxy, redirect, or auth behavior.
    pub client: Option<Client>,
    /// TIFF decoder options applied after range reads are assembled.
    pub tiff_options: TiffOpenOptions,
}

impl Default for HttpOpenOptions {
    fn default() -> Self {
        Self {
            chunk_size: 256 * 1024,
            cache_bytes: 64 * 1024 * 1024,
            cache_slots: 257,
            connect_timeout: Some(Duration::from_secs(10)),
            read_timeout: Some(Duration::from_secs(30)),
            request_timeout: Some(Duration::from_secs(120)),
            headers: HeaderMap::new(),
            client: None,
            tiff_options: TiffOpenOptions::default(),
        }
    }
}

/// Remote GeoTIFF/COG handle backed by HTTP range requests.
pub struct HttpGeoTiffFile {
    url: String,
    inner: GeoTiffFile,
}

impl HttpGeoTiffFile {
    /// Open a remote GeoTIFF/COG using HTTP range requests.
    pub fn open(url: impl Into<String>) -> Result<Self> {
        Self::open_with_options(url, HttpOpenOptions::default())
    }

    /// Open a remote GeoTIFF/COG using explicit range-cache options.
    pub fn open_with_options(url: impl Into<String>, options: HttpOpenOptions) -> Result<Self> {
        let url = url.into();
        let tiff_options = options.tiff_options;
        let source: SharedSource = Arc::new(HttpRangeSource::open(url.clone(), options)?);
        let tiff = TiffFile::from_source_with_options(source, tiff_options)?;
        let inner = GeoTiffFile::from_tiff(tiff)?;
        Ok(Self { url, inner })
    }

    /// The source URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Access the decoded GeoTIFF.
    pub fn inner(&self) -> &GeoTiffFile {
        &self.inner
    }
}

struct HttpRangeSource {
    client: Client,
    url: String,
    len: u64,
    chunk_size: usize,
    headers: HeaderMap,
    read_timeout: Option<Duration>,
    request_timeout: Option<Duration>,
    cache: Mutex<RangeCacheState>,
    max_bytes: usize,
    cache_enabled: bool,
}

struct RangeCacheState {
    cache: LruCache<u64, Arc<Vec<u8>>>,
    current_bytes: usize,
}

impl HttpRangeSource {
    fn open(url: String, options: HttpOpenOptions) -> Result<Self> {
        let client = build_client(&options)?;
        let len = probe_content_length(&client, &url, &options.headers, options.request_timeout)?;
        let slots = NonZeroUsize::new(options.cache_slots.max(1)).unwrap();
        Ok(Self {
            client,
            url,
            len,
            chunk_size: options.chunk_size.max(1),
            headers: options.headers,
            read_timeout: options.read_timeout,
            request_timeout: options.request_timeout,
            cache: Mutex::new(RangeCacheState {
                cache: LruCache::new(slots),
                current_bytes: 0,
            }),
            max_bytes: options.cache_bytes,
            cache_enabled: options.cache_bytes > 0 && options.cache_slots > 0,
        })
    }

    fn chunk(&self, index: u64) -> Result<Arc<Vec<u8>>> {
        if self.cache_enabled {
            let mut state = self.cache.lock();
            if let Some(chunk) = state.cache.get(&index) {
                return Ok(chunk.clone());
            }
        }

        let chunk_size = self.chunk_size as u64;
        let start = index
            .checked_mul(chunk_size)
            .ok_or_else(|| Error::Other("range chunk offset overflowed u64".into()))?;
        if start >= self.len {
            return Err(Error::Other(format!(
                "range chunk {index} starts beyond end of object"
            )));
        }
        let end = start.saturating_add(chunk_size).min(self.len) - 1;
        let response = request_with_options(
            self.client.get(&self.url),
            &self.headers,
            shorter_timeout(self.read_timeout, self.request_timeout),
        )
        .header(RANGE, format!("bytes={start}-{end}"))
        .send()?
        .error_for_status()?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(Error::Other(format!(
                "server did not honor byte-range request for {}: expected 206, got {}",
                self.url,
                response.status()
            )));
        }
        validate_content_range_header(
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok()),
            &self.url,
            start,
            end,
            Some(self.len),
        )?;
        let expected_len = usize::try_from(end - start + 1).unwrap_or(usize::MAX);
        let body = read_response_body(
            response,
            expected_len,
            self.request_timeout,
            &format!("{} bytes={start}-{end}", self.url),
        )?;
        if body.len() != expected_len {
            return Err(Error::Other(format!(
                "range response length mismatch for {}: expected {expected_len} bytes, got {}",
                self.url,
                body.len()
            )));
        }
        let body_len = body.len();
        let value = Arc::new(body);

        let mut state = self.cache.lock();
        if let Some(previous) = state.cache.pop(&index) {
            state.current_bytes = state.current_bytes.saturating_sub(previous.len());
        }

        if !self.cache_enabled || body_len > self.max_bytes {
            return Ok(value);
        }

        while state.current_bytes > self.max_bytes - body_len && !state.cache.is_empty() {
            if let Some((_, evicted)) = state.cache.pop_lru() {
                state.current_bytes = state.current_bytes.saturating_sub(evicted.len());
            }
        }
        state.current_bytes += body_len;
        if let Some((_, evicted)) = state.cache.push(index, value.clone()) {
            state.current_bytes = state.current_bytes.saturating_sub(evicted.len());
        }
        Ok(value)
    }
}

fn build_client(options: &HttpOpenOptions) -> Result<Client> {
    if let Some(client) = &options.client {
        return Ok(client.clone());
    }

    let mut builder = Client::builder();
    if let Some(timeout) = options.connect_timeout {
        builder = builder.connect_timeout(timeout);
    }
    if let Some(timeout) = options.request_timeout {
        builder = builder.timeout(timeout);
    }
    Ok(builder.build()?)
}

fn request_with_options(
    request: RequestBuilder,
    headers: &HeaderMap,
    request_timeout: Option<Duration>,
) -> RequestBuilder {
    let mut request = if headers.is_empty() {
        request
    } else {
        request.headers(headers.clone())
    };
    if let Some(timeout) = request_timeout {
        request = request.timeout(timeout);
    }
    request
}

fn shorter_timeout(lhs: Option<Duration>, rhs: Option<Duration>) -> Option<Duration> {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => Some(lhs.min(rhs)),
        (Some(timeout), None) | (None, Some(timeout)) => Some(timeout),
        (None, None) => None,
    }
}

fn read_response_body(
    mut response: reqwest::blocking::Response,
    expected_len: usize,
    request_timeout: Option<Duration>,
    context: &str,
) -> Result<Vec<u8>> {
    let started = Instant::now();
    let mut body = Vec::with_capacity(expected_len);
    let mut buffer = [0u8; 8192];

    while body.len() < expected_len {
        if let Some(timeout) = request_timeout {
            if started.elapsed() >= timeout {
                return Err(Error::Other(format!(
                    "HTTP response body read exceeded overall timeout for {context}"
                )));
            }
        }

        let remaining = expected_len - body.len();
        let read_len = buffer.len().min(remaining);
        let read = response
            .read(&mut buffer[..read_len])
            .map_err(|err| Error::Io(err, context.to_string()))?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..read]);
    }

    Ok(body)
}

impl TiffSource for HttpRangeSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&self, offset: u64, len: usize) -> tiff_reader::error::Result<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }

        let end = offset.checked_add(len as u64).ok_or({
            tiff_reader::TiffError::OffsetOutOfBounds {
                offset,
                length: len as u64,
                data_len: self.len,
            }
        })?;
        if end > self.len {
            return Err(tiff_reader::TiffError::OffsetOutOfBounds {
                offset,
                length: len as u64,
                data_len: self.len,
            });
        }

        let first_chunk = offset / self.chunk_size as u64;
        let last_chunk = (end.saturating_sub(1)) / self.chunk_size as u64;
        let mut out = Vec::with_capacity(len);

        for chunk_index in first_chunk..=last_chunk {
            let chunk = self.chunk(chunk_index).map_err(|e| {
                tiff_reader::TiffError::Other(format!("HTTP range read failed: {e}"))
            })?;
            let chunk_start = chunk_index * self.chunk_size as u64;
            let start_in_chunk = if chunk_index == first_chunk {
                usize::try_from(offset - chunk_start).unwrap_or(0)
            } else {
                0
            };
            let end_in_chunk = if chunk_index == last_chunk {
                usize::try_from(end - chunk_start).unwrap_or(chunk.len())
            } else {
                chunk.len()
            };
            out.extend_from_slice(&chunk[start_in_chunk..end_in_chunk]);
        }

        Ok(out)
    }
}

fn probe_content_length(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    request_timeout: Option<Duration>,
) -> Result<u64> {
    let head = request_with_options(client.head(url), headers, request_timeout).send()?;
    if head.status().is_success() {
        if let Some(value) = head.headers().get(CONTENT_LENGTH) {
            if let Ok(text) = value.to_str() {
                if let Ok(len) = text.parse::<u64>() {
                    return Ok(len);
                }
            }
        }
    }

    let response = request_with_options(client.get(url), headers, request_timeout)
        .header(RANGE, "bytes=0-0")
        .send()?
        .error_for_status()?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(Error::Other(format!(
            "server does not support HTTP range requests for {url}"
        )));
    }
    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| Error::Other(format!("missing Content-Range header for {url}")))?;
    probe_total_from_content_range(content_range, url)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use reqwest::blocking::Client;
    use reqwest::header::{HeaderMap, HeaderValue};
    use tiff_reader::source::TiffSource;

    use super::{HttpGeoTiffFile, HttpOpenOptions, HttpRangeSource};
    use crate::http_test_support::{build_simple_geotiff, TestServer};

    #[test]
    fn default_http_options_set_request_timeouts() {
        let options = HttpOpenOptions::default();

        assert_eq!(options.connect_timeout, Some(Duration::from_secs(10)));
        assert_eq!(options.read_timeout, Some(Duration::from_secs(30)));
        assert_eq!(options.request_timeout, Some(Duration::from_secs(120)));
        assert!(options.headers.is_empty());
        assert!(options.client.is_none());
    }

    #[test]
    fn opens_remote_geotiff_over_http_ranges() {
        let bytes = build_simple_geotiff();
        let Some(server) = TestServer::start(bytes) else {
            return;
        };
        let file = HttpGeoTiffFile::open_with_options(
            server.url(),
            HttpOpenOptions {
                chunk_size: 128,
                cache_bytes: 1024 * 1024,
                cache_slots: 16,
                ..HttpOpenOptions::default()
            },
        )
        .unwrap();

        assert_eq!(file.inner().epsg(), Some(4326));
        let raster = file.inner().read_raster::<u8>().unwrap();
        let (values, offset) = raster.into_raw_vec_and_offset();
        assert_eq!(offset, Some(0));
        assert_eq!(values, vec![10, 20, 30, 40]);
    }

    #[test]
    fn reads_real_cog_tile_bytes_exactly_over_small_ranges() {
        let Some(bytes) = real_cog_fixture() else {
            return;
        };
        let Some(server) = TestServer::start(bytes.clone()) else {
            return;
        };
        let source = HttpRangeSource::open(
            server.url(),
            HttpOpenOptions {
                chunk_size: 128,
                cache_bytes: 1024 * 1024,
                cache_slots: 16,
                ..HttpOpenOptions::default()
            },
        )
        .unwrap();

        let expected = &bytes[570..570 + 1223];
        let actual = source.read_exact_at(570, 1223).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn range_read_rejects_wrong_content_range_start_end() {
        let Some(server) = TestServer::start_with_content_range_offset(vec![0; 12], 1) else {
            return;
        };
        let source = HttpRangeSource::open(
            server.url(),
            HttpOpenOptions {
                chunk_size: 4,
                cache_bytes: 0,
                cache_slots: 0,
                ..HttpOpenOptions::default()
            },
        )
        .unwrap();

        let error = source.read_exact_at(0, 1).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("Content-Range"), "{message}");
        assert!(message.contains("expected bytes 0-3"), "{message}");
    }

    #[test]
    fn sends_custom_headers_with_custom_client_for_probe_and_range_requests() {
        let Some(server) = TestServer::start(vec![0; 12]) else {
            return;
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-test-auth", HeaderValue::from_static("secret"));
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let source = HttpRangeSource::open(
            server.url(),
            HttpOpenOptions {
                chunk_size: 4,
                cache_bytes: 0,
                cache_slots: 0,
                headers,
                client: Some(client),
                ..HttpOpenOptions::default()
            },
        )
        .unwrap();

        source.read_exact_at(0, 1).unwrap();

        let requests = server.requests();
        assert!(
            requests.iter().any(|request| {
                request.starts_with("HEAD ")
                    && request.to_ascii_lowercase().contains("x-test-auth: secret")
            }),
            "HEAD request did not include custom header: {requests:?}"
        );
        assert!(
            requests.iter().any(|request| {
                let lower = request.to_ascii_lowercase();
                request.starts_with("GET ")
                    && lower.contains("range: bytes=0-3")
                    && lower.contains("x-test-auth: secret")
            }),
            "range GET request did not include custom header: {requests:?}"
        );
    }

    #[test]
    fn range_cache_slot_eviction_updates_byte_accounting() {
        let Some(server) = TestServer::start(vec![0; 12]) else {
            return;
        };
        let source = HttpRangeSource::open(
            server.url(),
            HttpOpenOptions {
                chunk_size: 4,
                cache_bytes: 100,
                cache_slots: 2,
                ..HttpOpenOptions::default()
            },
        )
        .unwrap();

        source.chunk(0).unwrap();
        source.chunk(1).unwrap();
        source.chunk(2).unwrap();

        assert_eq!(source.cache.lock().current_bytes, 8);
    }

    #[test]
    fn zero_range_cache_slots_disable_storage() {
        let Some(server) = TestServer::start(vec![0; 12]) else {
            return;
        };
        let source = HttpRangeSource::open(
            server.url(),
            HttpOpenOptions {
                chunk_size: 4,
                cache_bytes: 100,
                cache_slots: 0,
                ..HttpOpenOptions::default()
            },
        )
        .unwrap();

        source.chunk(0).unwrap();

        assert_eq!(source.cache.lock().current_bytes, 0);
    }

    #[test]
    fn zero_length_range_read_does_not_fetch_chunk() {
        let Some(server) = TestServer::start(vec![0; 12]) else {
            return;
        };
        let source = HttpRangeSource::open(
            server.url(),
            HttpOpenOptions {
                chunk_size: 4,
                cache_bytes: 100,
                cache_slots: 2,
                ..HttpOpenOptions::default()
            },
        )
        .unwrap();

        assert_eq!(source.read_exact_at(12, 0).unwrap(), Vec::<u8>::new());
        assert_eq!(source.cache.lock().current_bytes, 0);
    }

    fn real_cog_fixture() -> Option<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../testdata/interoperability/gdal/gcore/data/cog/byte_little_endian_golden.tif");
        std::fs::read(path).ok()
    }
}
