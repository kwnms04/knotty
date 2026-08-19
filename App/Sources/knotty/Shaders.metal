#include <metal_stdlib>

using namespace metal;

// The two passes land with the first pixel. This pair exists so the assembly
// script's `xcrun metal` step is a real check from the start rather than one
// nothing has ever compiled.

vertex float4 knotty_vertex(uint vertex_id [[vertex_id]]) {
    return float4(0.0, 0.0, 0.0, 1.0);
}

fragment float4 knotty_fragment() {
    return float4(0.0, 0.0, 0.0, 1.0);
}
