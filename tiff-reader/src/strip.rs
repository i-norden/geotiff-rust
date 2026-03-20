//! Strip-based data access for TIFF images.

// TODO: implement strip reading
// - parse StripOffsets (tag 273) and StripByteCounts (tag 279)
// - read raw strip data from file
// - apply decompression pipeline
// - assemble strips into contiguous pixel buffer
