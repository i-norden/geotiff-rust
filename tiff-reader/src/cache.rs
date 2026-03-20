//! LRU cache for decompressed strips and tiles.

// TODO: implement tile/strip cache
// - mirror hdf5-reader's ChunkCache pattern
// - configurable max bytes and slot count
// - key by (ifd_index, strip/tile_index)
