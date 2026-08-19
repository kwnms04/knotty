// Snapshot + metrics -> instance buffers and atlas updates. The renderer
// itself arrives with the first drawing; the target stands now so the
// boundary it enforces — no AppKit, no GPU device, no C boundary — is in
// place before there is code to break it. cf. 04-renderer R9.
