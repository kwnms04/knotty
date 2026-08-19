#include <metal_stdlib>

using namespace metal;

// Two passes, a draw call each: cell rectangles, then atlas quads tinted with
// the cell's foreground. cf. 04-renderer R1.

/// One instance of either pass: a rectangle and a colour.
///
/// The background pass reads `geometry` as the rectangle itself. The glyph
/// pass reads it as the cell's origin and the atlas origin, because a glyph
/// quad is always exactly its cell — which is why nothing here carries a size.
struct Instance {
    float4 geometry;
    float4 color;
};

struct Uniforms {
    /// The drawable, in device pixels.
    float2 viewport;
    /// One cell, in device pixels.
    float2 cell;
    /// A page side, in device pixels.
    float atlasSide;
};

struct Varying {
    float4 position [[position]];
    float4 color;
    float2 texture;
};

/// A unit quad as a triangle strip, so neither pass needs a vertex buffer.
constant float2 corners[4] = {float2(0, 0), float2(1, 0), float2(0, 1), float2(1, 1)};

/// Device pixels with the origin at the top left — which is where the renderer
/// puts everything — into clip space.
static float4 clip(float2 pixel, float2 viewport) {
    return float4(pixel.x / viewport.x * 2.0 - 1.0, 1.0 - pixel.y / viewport.y * 2.0, 0.0, 1.0);
}

vertex Varying knotty_background_vertex(
    uint vertex_id [[vertex_id]],
    uint instance_id [[instance_id]],
    constant Instance *instances [[buffer(0)]],
    constant Uniforms &uniforms [[buffer(1)]]
) {
    const Instance instance = instances[instance_id];
    Varying out;
    out.position = clip(
        instance.geometry.xy + corners[vertex_id] * instance.geometry.zw, uniforms.viewport
    );
    out.color = instance.color;
    out.texture = float2(0);
    return out;
}

fragment float4 knotty_background_fragment(Varying in [[stage_in]]) {
    return in.color;
}

vertex Varying knotty_glyph_vertex(
    uint vertex_id [[vertex_id]],
    uint instance_id [[instance_id]],
    constant Instance *instances [[buffer(0)]],
    constant Uniforms &uniforms [[buffer(1)]]
) {
    const Instance instance = instances[instance_id];
    const float2 corner = corners[vertex_id];
    Varying out;
    out.position = clip(instance.geometry.xy + corner * uniforms.cell, uniforms.viewport);
    out.color = instance.color;
    out.texture = (instance.geometry.zw + corner * uniforms.cell) / uniforms.atlasSide;
    return out;
}

fragment float4 knotty_glyph_fragment(
    Varying in [[stage_in]], texture2d<float> atlas [[texture(0)]]
) {
    // Nearest, because a quad is its cell and a slot is its cell: every fetch
    // lands on a texel centre, and there is nothing for a filter to average.
    constexpr sampler texel(filter::nearest, coord::normalized, address::clamp_to_edge);
    // The page holds coverage, and the colour comes from the cell — which is
    // what keeps one raster good for every colour it is ever drawn in.
    // cf. 04-renderer R8.
    return float4(in.color.rgb, in.color.a * atlas.sample(texel, in.texture).r);
}
