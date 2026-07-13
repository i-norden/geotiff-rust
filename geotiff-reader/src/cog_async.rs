//! Asynchronous HTTP range-backed remote GeoTIFF/COG access.
//!
//! Range requests use the async `reqwest` client while TIFF parsing and
//! block decoding run on the Tokio blocking pool, bridged through a
//! [`TiffSource`] whose reads block on in-flight range fetches. All decode
//! entry points on [`AsyncHttpGeoTiffFile`] are `async`; metadata accessors
//! are synchronous because everything they need is resolved at open time.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use lru::LruCache;
use parking_lot::Mutex;
use reqwest::header::{HeaderMap, CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use reqwest::{Client, RequestBuilder, StatusCode};
use tiff_reader::source::{SharedSource, TiffSource};
use tiff_reader::{OpenOptions as TiffOpenOptions, TiffFile, TiffSample};
use tokio::runtime::Handle;

use crate::http_range::{probe_total_from_content_range, validate_content_range_header};
use crate::{Error, GeoTiffFile, Result};

/// Options for asynchronous HTTP range-backed GeoTIFF access.
#[derive(Debug, Clone)]
pub struct AsyncHttpOpenOptions {
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
    /// Overall timeout applied to each HEAD or GET request, including the
    /// response body.
    pub request_timeout: Option<Duration>,
    /// Headers sent on every HEAD and byte-range GET request.
    pub headers: HeaderMap,
    /// Optional preconfigured async client for custom TLS, proxy, redirect,
    /// or auth behavior.
    pub client: Option<Client>,
    /// TIFF decoder options applied after range reads are assembled.
    pub tiff_options: TiffOpenOptions,
}

impl Default for AsyncHttpOpenOptions {
    fn default() -> Self {
        Self {
            chunk_size: 256 * 1024,
            cache_bytes: 64 * 1024 * 1024,
            cache_slots: 257,
            connect_timeout: Some(Duration::from_secs(10)),
            request_timeout: Some(Duration::from_secs(120)),
            headers: HeaderMap::new(),
            client: None,
            tiff_options: TiffOpenOptions::default(),
        }
    }
}

/// Remote GeoTIFF/COG handle backed by asynchronous HTTP range requests.
pub struct AsyncHttpGeoTiffFile {
    url: String,
    inner: Arc<GeoTiffFile>,
}

impl AsyncHttpGeoTiffFile {
    /// Open a remote GeoTIFF/COG using asynchronous HTTP range requests.
    ///
    /// Must be called within a multi-thread Tokio runtime.
    pub async fn open(url: impl Into<String>) -> Result<Self> {
        Self::open_with_options(url, AsyncHttpOpenOptions::default()).await
    }

    /// Open a remote GeoTIFF/COG using explicit range-cache options.
    pub async fn open_with_options(
        url: impl Into<String>,
        options: AsyncHttpOpenOptions,
    ) -> Result<Self> {
        let url = url.into();
        let tiff_options = options.tiff_options;
        let source = Arc::new(AsyncHttpRangeSource::open(url.clone(), options).await?);
        let bridged: SharedSource = Arc::new(BridgedTiffSource {
            source,
            handle: Handle::current(),
        });
        let inner = spawn_decode(move || {
            let tiff = TiffFile::from_source_with_options(bridged, tiff_options)?;
            GeoTiffFile::from_tiff(tiff)
        })
        .await?;
        Ok(Self {
            url,
            inner: Arc::new(inner),
        })
    }

    /// The source URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Access the decoded GeoTIFF for synchronous metadata queries.
    ///
    /// Only metadata accessors are safe here: the inner file's blocking read
    /// methods bridge into the async runtime and panic when called from an
    /// async context. Use the `read_*` methods on this type instead.
    pub fn inner(&self) -> &GeoTiffFile {
        &self.inner
    }

    /// Decode the base-resolution raster into storage-domain typed samples.
    pub async fn read_raster<T: TiffSample + Send>(&self) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || inner.read_raster::<T>()).await
    }

    /// Decode the base-resolution raster into color-decoded typed pixels.
    pub async fn read_decoded_raster<T: TiffSample + Send>(&self) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || inner.read_decoded_raster::<T>()).await
    }

    /// Decode a base-resolution pixel window into storage-domain typed samples.
    pub async fn read_window<T: TiffSample + Send>(
        &self,
        row_off: usize,
        col_off: usize,
        rows: usize,
        cols: usize,
    ) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || inner.read_window::<T>(row_off, col_off, rows, cols)).await
    }

    /// Decode a base-resolution pixel window into color-decoded typed pixels.
    pub async fn read_decoded_window<T: TiffSample + Send>(
        &self,
        row_off: usize,
        col_off: usize,
        rows: usize,
        cols: usize,
    ) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || inner.read_decoded_window::<T>(row_off, col_off, rows, cols)).await
    }

    /// Decode one base-resolution storage-domain band.
    pub async fn read_band<T: TiffSample + Send>(
        &self,
        band_index: usize,
    ) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || inner.read_band::<T>(band_index)).await
    }

    /// Decode a base-resolution window from one storage-domain band.
    pub async fn read_band_window<T: TiffSample + Send>(
        &self,
        band_index: usize,
        row_off: usize,
        col_off: usize,
        rows: usize,
        cols: usize,
    ) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || inner.read_band_window::<T>(band_index, row_off, col_off, rows, cols))
            .await
    }

    /// Decode an overview raster into storage-domain typed samples.
    pub async fn read_overview<T: TiffSample + Send>(
        &self,
        overview_index: usize,
    ) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || inner.read_overview::<T>(overview_index)).await
    }

    /// Decode an overview pixel window into storage-domain typed samples.
    pub async fn read_overview_window<T: TiffSample + Send>(
        &self,
        overview_index: usize,
        row_off: usize,
        col_off: usize,
        rows: usize,
        cols: usize,
    ) -> Result<ndarray::ArrayD<T>> {
        let inner = Arc::clone(&self.inner);
        spawn_decode(move || {
            inner.read_overview_window::<T>(overview_index, row_off, col_off, rows, cols)
        })
        .await
    }
}

async fn spawn_decode<T, F>(decode: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(decode)
        .await
        .map_err(|error| Error::Other(format!("decode task failed: {error}")))?
}

/// Sync [`TiffSource`] facade over the async range source.
///
/// Reads block on the captured runtime handle, which is only legal from the
/// Tokio blocking pool; every decode entry point above routes through
/// `spawn_blocking` to guarantee that.
struct BridgedTiffSource {
    source: Arc<AsyncHttpRangeSource>,
    handle: Handle,
}

impl TiffSource for BridgedTiffSource {
    fn len(&self) -> u64 {
        self.source.len
    }

    fn read_exact_at(&self, offset: u64, len: usize) -> tiff_reader::error::Result<Vec<u8>> {
        let source = Arc::clone(&self.source);
        self.handle
            .block_on(async move { source.read_exact_at(offset, len).await })
    }
}

struct AsyncHttpRangeSource {
    client: Client,
    url: String,
    len: u64,
    chunk_size: usize,
    headers: HeaderMap,
    request_timeout: Option<Duration>,
    cache: Mutex<RangeCacheState>,
    max_bytes: usize,
    cache_enabled: bool,
}

struct RangeCacheState {
    cache: LruCache<u64, Arc<Vec<u8>>>,
    current_bytes: usize,
}

impl AsyncHttpRangeSource {
    async fn open(url: String, options: AsyncHttpOpenOptions) -> Result<Self> {
        let client = match &options.client {
            Some(client) => client.clone(),
            None => {
                let mut builder = Client::builder();
                if let Some(timeout) = options.connect_timeout {
                    builder = builder.connect_timeout(timeout);
                }
                if let Some(timeout) = options.request_timeout {
                    builder = builder.timeout(timeout);
                }
                builder.build()?
            }
        };
        let len =
            probe_content_length(&client, &url, &options.headers, options.request_timeout).await?;
        let slots = NonZeroUsize::new(options.cache_slots.max(1)).unwrap();
        Ok(Self {
            client,
            url,
            len,
            chunk_size: options.chunk_size.max(1),
            headers: options.headers,
            request_timeout: options.request_timeout,
            cache: Mutex::new(RangeCacheState {
                cache: LruCache::new(slots),
                current_bytes: 0,
            }),
            max_bytes: options.cache_bytes,
            cache_enabled: options.cache_bytes > 0 && options.cache_slots > 0,
        })
    }

    async fn chunk(&self, index: u64) -> Result<Arc<Vec<u8>>> {
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
            self.request_timeout,
        )
        .header(RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?
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
        let body = read_response_body_bounded(
            response,
            expected_len,
            &format!("{} bytes={start}-{end}", self.url),
        )
        .await?;
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

    async fn read_exact_at(&self, offset: u64, len: usize) -> tiff_reader::error::Result<Vec<u8>> {
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
            let chunk = self.chunk(chunk_index).await.map_err(|e| {
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

async fn read_response_body_bounded(
    mut response: reqwest::Response,
    expected_len: usize,
    context: &str,
) -> Result<Vec<u8>> {
    if let Some(content_len) = response.content_length() {
        let expected_len_u64 = u64::try_from(expected_len).unwrap_or(u64::MAX);
        if content_len > expected_len_u64 {
            return Err(Error::Other(format!(
                "HTTP response body for {context} exceeds the expected {expected_len}-byte range"
            )));
        }
    }

    let mut body = Vec::with_capacity(expected_len);
    while let Some(chunk) = response.chunk().await? {
        let remaining = expected_len - body.len();
        if chunk.len() > remaining {
            return Err(Error::Other(format!(
                "HTTP response body for {context} exceeds the expected {expected_len}-byte range"
            )));
        }
        body.extend_from_slice(&chunk);
    }

    if body.len() != expected_len {
        return Err(Error::Other(format!(
            "range response length mismatch for {context}: expected {expected_len} bytes, got {}",
            body.len()
        )));
    }
    Ok(body)
}

async fn probe_content_length(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    request_timeout: Option<Duration>,
) -> Result<u64> {
    let head = request_with_options(client.head(url), headers, request_timeout)
        .send()
        .await?;
    if head.status().is_success() {
        if let Some(len) = head
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse::<u64>().ok())
        {
            return Ok(len);
        }
    }

    let response = request_with_options(client.get(url), headers, request_timeout)
        .header(RANGE, "bytes=0-0")
        .send()
        .await?
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
    use std::time::{Duration, Instant};

    use reqwest::Client;

    use super::{AsyncHttpGeoTiffFile, AsyncHttpOpenOptions, AsyncHttpRangeSource};
    use crate::http_test_support::{build_simple_geotiff, TestServer};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn opens_remote_geotiff_over_async_http_ranges() {
        let bytes = build_simple_geotiff();
        let Some(server) = TestServer::start(bytes) else {
            return;
        };

        let file = AsyncHttpGeoTiffFile::open_with_options(
            server.url(),
            AsyncHttpOpenOptions {
                chunk_size: 128,
                cache_bytes: 1024 * 1024,
                cache_slots: 16,
                ..AsyncHttpOpenOptions::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(file.inner().epsg(), Some(4326));
        assert_eq!(file.inner().nodata(), Some("-9999"));

        let raster = file.read_raster::<u8>().await.unwrap();
        let (values, offset) = raster.into_raw_vec_and_offset();
        assert_eq!(offset, Some(0));
        assert_eq!(values, vec![10, 20, 30, 40]);

        let window = file.read_window::<u8>(1, 0, 1, 2).await.unwrap();
        let (values, offset) = window.into_raw_vec_and_offset();
        assert_eq!(offset, Some(0));
        assert_eq!(values, vec![30, 40]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_range_reads_send_custom_headers() {
        let Some(server) = TestServer::start(vec![0; 12]) else {
            return;
        };
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-test-auth",
            reqwest::header::HeaderValue::from_static("secret"),
        );

        // 12 zero bytes are not a TIFF; opening fails after the probe and
        // first range request, which is all this test needs.
        let result = AsyncHttpGeoTiffFile::open_with_options(
            server.url(),
            AsyncHttpOpenOptions {
                chunk_size: 4,
                headers,
                ..AsyncHttpOpenOptions::default()
            },
        )
        .await;
        assert!(result.is_err());

        let requests = server.requests();
        assert!(requests
            .iter()
            .any(|request| request.to_ascii_lowercase().contains("x-test-auth: secret")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn custom_async_client_honors_request_timeout() {
        let delay = Duration::from_millis(500);
        let Some(server) = TestServer::start_with_response_delay(vec![0; 12], delay) else {
            return;
        };
        let started = Instant::now();
        let result = AsyncHttpGeoTiffFile::open_with_options(
            server.url(),
            AsyncHttpOpenOptions {
                request_timeout: Some(Duration::from_millis(30)),
                client: Some(Client::builder().build().unwrap()),
                ..AsyncHttpOpenOptions::default()
            },
        )
        .await;

        assert!(result.is_err());
        assert!(started.elapsed() < delay, "custom client ignored timeout");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_range_read_rejects_oversized_body_before_buffering_it() {
        let Some(server) =
            TestServer::start_with_range_body_suffix(vec![0; 12], vec![1; 1024 * 1024])
        else {
            return;
        };
        let source = AsyncHttpRangeSource::open(
            server.url(),
            AsyncHttpOpenOptions {
                chunk_size: 4,
                cache_bytes: 0,
                cache_slots: 0,
                ..AsyncHttpOpenOptions::default()
            },
        )
        .await
        .unwrap();

        let error = source.chunk(0).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("exceeds the expected 4-byte range"));
    }
}
