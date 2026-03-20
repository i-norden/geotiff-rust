//! Tile-based data access for TIFF images (used by COG).

// TODO: implement tile reading
// - parse TileOffsets (tag 324) and TileByteCounts (tag 325)
// - read raw tile data from file
// - apply decompression pipeline
// - assemble tiles into contiguous pixel buffer
// - support parallel tile decompression via rayon
