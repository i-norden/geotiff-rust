#![no_main]

use geotiff_reader::GeoTiffFile;
use libfuzzer_sys::fuzz_target;

const MAX_SYNTHETIC_SUBIFDS: usize = 128;
const MAX_CHILDREN_PER_IFD: usize = 4;
const MAX_OVERVIEWS_TO_READ: usize = 8;

#[derive(Clone)]
struct IfdSpec {
    entries: Vec<(u16, u16, u32, Vec<u8>)>,
    image_data: Vec<u8>,
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let bytes = build_subifd_geotiff(data);
    let file = match GeoTiffFile::from_bytes(bytes) {
        Ok(file) => file,
        Err(_) => return,
    };

    for overview_index in 0..file.overview_count().min(MAX_OVERVIEWS_TO_READ) {
        let _ = file.overview_ifd_index(overview_index);
        let _ = file.overview_ifd(overview_index);
        let _ = file.read_overview::<u8>(overview_index);
    }
});

fn build_subifd_geotiff(data: &[u8]) -> Vec<u8> {
    let node_count = usize::from(data[0])
        .min(MAX_SYNTHETIC_SUBIFDS - 1)
        .saturating_add(1);
    let mut cursor = ByteCursor::new(&data[1..]);
    let graph = SubIfdGraph::from_bytes(node_count, &mut cursor);

    let mut ifds = Vec::with_capacity(node_count + 1);
    ifds.push(base_ifd(graph.root_children.len()));
    for children in &graph.node_children {
        ifds.push(overview_ifd(children.len()));
    }

    let mut bytes = build_classic_tiff(&ifds);
    let node_offsets = top_level_ifd_offsets_after_first(&bytes, node_count);

    overwrite_long_tag_values(
        &mut bytes,
        8,
        330,
        &graph
            .root_children
            .iter()
            .map(|index| node_offsets[*index])
            .collect::<Vec<_>>(),
    );
    for (index, children) in graph.node_children.iter().enumerate() {
        if children.is_empty() {
            continue;
        }
        overwrite_long_tag_values(
            &mut bytes,
            node_offsets[index] as usize,
            330,
            &children
                .iter()
                .map(|child_index| node_offsets[*child_index])
                .collect::<Vec<_>>(),
        );
    }
    overwrite_first_ifd_next_pointer(&mut bytes, 0);
    bytes
}

struct SubIfdGraph {
    root_children: Vec<usize>,
    node_children: Vec<Vec<usize>>,
}

impl SubIfdGraph {
    fn from_bytes(node_count: usize, cursor: &mut ByteCursor<'_>) -> Self {
        let root_count = cursor.next_usize(MAX_CHILDREN_PER_IFD.min(node_count)) + 1;
        let root_children = (0..root_count)
            .map(|_| cursor.next_usize(node_count))
            .collect();

        let node_children = (0..node_count)
            .map(|_| {
                let child_count = cursor.next_usize(MAX_CHILDREN_PER_IFD + 1);
                (0..child_count)
                    .map(|_| cursor.next_usize(node_count))
                    .collect()
            })
            .collect();

        Self {
            root_children,
            node_children,
        }
    }
}

struct ByteCursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> ByteCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }

    fn next_usize(&mut self, modulo: usize) -> usize {
        if modulo == 0 {
            return 0;
        }
        let byte = self.bytes.get(self.index).copied().unwrap_or(0);
        self.index = self.index.saturating_add(1);
        usize::from(byte) % modulo
    }
}

fn base_ifd(subifd_count: usize) -> IfdSpec {
    let geo_keys = [1u16, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, 4326];
    let mut entries = vec![
        (256u16, 4u16, 1u32, le_u32(64).to_vec()),
        (257u16, 4u16, 1u32, le_u32(64).to_vec()),
        (258u16, 3u16, 1u32, inline_short(8)),
        (259u16, 3u16, 1u32, inline_short(1)),
        (273u16, 4u16, 1u32, vec![]),
        (277u16, 3u16, 1u32, inline_short(1)),
        (278u16, 4u16, 1u32, le_u32(64).to_vec()),
        (279u16, 4u16, 1u32, le_u32(1).to_vec()),
    ];
    if subifd_count > 0 {
        entries.push((
            330u16,
            4u16,
            subifd_count as u32,
            vec![0; subifd_count * 4],
        ));
    }
    entries.extend([
        (
            33550u16,
            12u16,
            3u32,
            [2.0, 2.0, 0.0]
                .iter()
                .flat_map(|value| le_f64(*value))
                .collect(),
        ),
        (
            33922u16,
            12u16,
            6u32,
            [0.0, 0.0, 0.0, 100.0, 200.0, 0.0]
                .iter()
                .flat_map(|value| le_f64(*value))
                .collect(),
        ),
        (
            34735u16,
            3u16,
            geo_keys.len() as u32,
            geo_keys.iter().flat_map(|value| le_u16(*value)).collect(),
        ),
    ]);

    IfdSpec {
        entries,
        image_data: vec![0u8],
    }
}

fn overview_ifd(subifd_count: usize) -> IfdSpec {
    let mut entries = vec![
        (254u16, 4u16, 1u32, le_u32(1).to_vec()),
        (256u16, 4u16, 1u32, le_u32(1).to_vec()),
        (257u16, 4u16, 1u32, le_u32(1).to_vec()),
        (258u16, 3u16, 1u32, inline_short(8)),
        (259u16, 3u16, 1u32, inline_short(1)),
        (273u16, 4u16, 1u32, vec![]),
        (277u16, 3u16, 1u32, inline_short(1)),
        (278u16, 4u16, 1u32, le_u32(1).to_vec()),
        (279u16, 4u16, 1u32, le_u32(1).to_vec()),
    ];
    if subifd_count > 0 {
        entries.push((
            330u16,
            4u16,
            subifd_count as u32,
            vec![0; subifd_count * 4],
        ));
    }

    IfdSpec {
        entries,
        image_data: vec![0u8],
    }
}

fn build_classic_tiff(ifds: &[IfdSpec]) -> Vec<u8> {
    let mut ifd_offsets = Vec::with_capacity(ifds.len());
    let mut cursor = 8usize;
    for ifd in ifds {
        ifd_offsets.push(cursor as u32);
        let deferred_len: usize = ifd
            .entries
            .iter()
            .filter(|(tag, _, _, value)| *tag != 273 && value.len() > 4)
            .map(|(_, _, _, value)| value.len())
            .sum();
        cursor += 2 + ifd.entries.len() * 12 + 4 + ifd.image_data.len() + deferred_len;
    }

    let mut bytes = Vec::with_capacity(cursor);
    bytes.extend_from_slice(b"II");
    bytes.extend_from_slice(&le_u16(42));
    bytes.extend_from_slice(&le_u32(ifd_offsets.first().copied().unwrap_or(0)));

    for (ifd_index, ifd) in ifds.iter().enumerate() {
        let ifd_offset = ifd_offsets[ifd_index] as usize;
        debug_assert_eq!(bytes.len(), ifd_offset);

        let ifd_size = 2 + ifd.entries.len() * 12 + 4;
        let mut next_data_offset = ifd_offset + ifd_size;
        let image_offset = next_data_offset as u32;
        next_data_offset += ifd.image_data.len();

        bytes.extend_from_slice(&le_u16(ifd.entries.len() as u16));
        let mut deferred = Vec::new();
        for (tag, ty, count, value) in &ifd.entries {
            bytes.extend_from_slice(&le_u16(*tag));
            bytes.extend_from_slice(&le_u16(*ty));
            bytes.extend_from_slice(&le_u32(*count));
            if *tag == 273 {
                bytes.extend_from_slice(&le_u32(image_offset));
            } else if value.len() <= 4 {
                let mut inline = [0u8; 4];
                inline[..value.len()].copy_from_slice(value);
                bytes.extend_from_slice(&inline);
            } else {
                bytes.extend_from_slice(&le_u32(next_data_offset as u32));
                next_data_offset += value.len();
                deferred.push(value.clone());
            }
        }

        let next_ifd_offset = ifd_offsets.get(ifd_index + 1).copied().unwrap_or(0);
        bytes.extend_from_slice(&le_u32(next_ifd_offset));
        bytes.extend_from_slice(&ifd.image_data);
        for value in deferred {
            bytes.extend_from_slice(&value);
        }
        debug_assert_eq!(bytes.len(), next_data_offset);
    }

    bytes
}

fn overwrite_long_tag_values(bytes: &mut [u8], ifd_offset: usize, tag_code: u16, values: &[u32]) {
    if values.is_empty() {
        return;
    }

    let entry_count = u16::from_le_bytes([bytes[ifd_offset], bytes[ifd_offset + 1]]) as usize;
    let mut offset = ifd_offset + 2;
    for _ in 0..entry_count {
        let code = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        if code == tag_code {
            if values.len() == 1 {
                bytes[offset + 8..offset + 12].copy_from_slice(&le_u32(values[0]));
            } else {
                let value_offset = u32::from_le_bytes([
                    bytes[offset + 8],
                    bytes[offset + 9],
                    bytes[offset + 10],
                    bytes[offset + 11],
                ]) as usize;
                for (index, value) in values.iter().enumerate() {
                    let value_offset = value_offset + index * 4;
                    bytes[value_offset..value_offset + 4].copy_from_slice(&le_u32(*value));
                }
            }
            return;
        }
        offset += 12;
    }
}

fn overwrite_first_ifd_next_pointer(bytes: &mut [u8], value: u32) {
    let entry_count = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let pointer_offset = 10 + entry_count * 12;
    bytes[pointer_offset..pointer_offset + 4].copy_from_slice(&le_u32(value));
}

fn top_level_ifd_offsets_after_first(bytes: &[u8], count: usize) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(count);
    let mut offset = first_ifd_next_pointer(bytes);
    while offset != 0 && offsets.len() < count {
        offsets.push(offset);
        offset = ifd_next_pointer(bytes, offset as usize);
    }
    offsets
}

fn first_ifd_next_pointer(bytes: &[u8]) -> u32 {
    let entry_count = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let pointer_offset = 10 + entry_count * 12;
    u32::from_le_bytes([
        bytes[pointer_offset],
        bytes[pointer_offset + 1],
        bytes[pointer_offset + 2],
        bytes[pointer_offset + 3],
    ])
}

fn ifd_next_pointer(bytes: &[u8], ifd_offset: usize) -> u32 {
    let entry_count = u16::from_le_bytes([bytes[ifd_offset], bytes[ifd_offset + 1]]) as usize;
    let pointer_offset = ifd_offset + 2 + entry_count * 12;
    u32::from_le_bytes([
        bytes[pointer_offset],
        bytes[pointer_offset + 1],
        bytes[pointer_offset + 2],
        bytes[pointer_offset + 3],
    ])
}

fn inline_short(value: u16) -> Vec<u8> {
    let mut bytes = [0u8; 4];
    bytes[..2].copy_from_slice(&le_u16(value));
    bytes.to_vec()
}

fn le_u16(value: u16) -> [u8; 2] {
    value.to_le_bytes()
}

fn le_u32(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

fn le_f64(value: f64) -> [u8; 8] {
    value.to_le_bytes()
}
