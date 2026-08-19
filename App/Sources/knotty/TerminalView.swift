import AppKit
import Metal
import QuartzCore
import os

import KnottyRender
import KnottySession

/// The view the terminal appears in: the owner of the beat it appears on, and
/// of the Metal layer it appears in.
///
/// The loop is the one 05-swift-app 6 describes — a wake resumes the link, a
/// tick takes the frame and draws it, and a tick with nothing behind it stops
/// the link again. What the drawing does is carry: the renderer already said
/// where every rectangle and every quad goes, and nothing here decides
/// anything about the screen. cf. 04-renderer R9.
final class TerminalView: NSView {
    /// Whose frames these are.
    ///
    /// Weak, because the window controller owns it and quitting has to be able
    /// to release it while the window is still up: releasing the session is
    /// what puts the child down.
    private weak var host: SessionHost?

    /// Raised by the core's thread, lowered by a tick. The whole of what the
    /// wake callback is allowed to touch.
    private let pending = Pending()

    private var link: CADisplayLink?

    /// The grid, in device pixels. The window does not resize and the metrics
    /// do not move, so this is measured once and is the drawable's size for
    /// as long as the view lives.
    private let pixels: CGSize
    private let metrics: CellMetrics

    private let queue: MTLCommandQueue
    private let backgroundPipeline: MTLRenderPipelineState
    private let glyphPipeline: MTLRenderPipelineState
    /// The one A8 page, filled in as the renderer bakes into it.
    private let atlas: MTLTexture
    /// What both passes need and neither instance carries.
    private let uniforms: Uniforms

    init(host: SessionHost, metrics: CellMetrics, pixels: CGSize, scale: Double) throws {
        self.host = host
        self.metrics = metrics
        self.pixels = pixels

        guard let device = MTLCreateSystemDefaultDevice() else {
            throw MetalMissing("a GPU")
        }
        guard let queue = device.makeCommandQueue() else {
            throw MetalMissing("a command queue")
        }
        // The bundle's `default.metallib`, which the assembly script compiles
        // and copies in. A bare `swift build` leaves none, which is the other
        // half of why the run path is the `.app`. cf. adr/0014.
        guard let library = device.makeDefaultLibrary() else {
            throw MetalMissing("the shader library")
        }
        self.queue = queue
        backgroundPipeline = try Self.pipeline(
            device: device, library: library,
            vertex: "knotty_background_vertex", fragment: "knotty_background_fragment",
            blending: false
        )
        // The glyph pass blends: a quad is a whole cell and a letter is not,
        // so what the page says is uncovered has to leave the background
        // showing.
        glyphPipeline = try Self.pipeline(
            device: device, library: library,
            vertex: "knotty_glyph_vertex", fragment: "knotty_glyph_fragment",
            blending: true
        )

        let side = Int(Renderer.atlasSide)
        let description = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .r8Unorm, width: side, height: side, mipmapped: false
        )
        description.usage = .shaderRead
        guard let atlas = device.makeTexture(descriptor: description) else {
            throw MetalMissing("a \(side)x\(side) atlas page")
        }
        self.atlas = atlas

        uniforms = Uniforms(
            viewport: SIMD2(Float(pixels.width), Float(pixels.height)),
            cell: SIMD2(Float(metrics.width), Float(metrics.height)),
            atlasSide: Float(side)
        )

        super.init(frame: .zero)

        // What asks `makeBackingLayer()` for the layer configured just below.
        wantsLayer = true
        guard let layer = layer as? CAMetalLayer else {
            throw MetalMissing("a Metal layer")
        }
        layer.device = device
        // Not the sRGB pair: a cell's colour is already an sRGB byte, and the
        // format that encodes on write would need it linear first.
        layer.pixelFormat = .bgra8Unorm
        // Which says what those bytes mean. An untagged layer hands them to
        // the display untouched, and on a wide-gamut panel that is every
        // colour a shade louder than the one the terminal asked for.
        layer.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        layer.contentsScale = scale

        // All the core's thread does is raise the flag, and it crosses to main
        // only on the raise that changed it. N wakes become one block, which
        // is the coalescing the boundary already does, carried on to this side.
        try host.onWake { [pending, weak self] in
            guard pending.raise() else { return }
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.link?.isPaused = false }
            }
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("knotty builds its views in code")
    }

    override func makeBackingLayer() -> CALayer { CAMetalLayer() }

    /// The drawable is the grid to the pixel, rather than the bounds times the
    /// scale — so no rounding sits between a cell and a texel.
    ///
    /// Here and not in the initializer because a Metal layer derives that
    /// product again on every bounds change, and the view is sized after it is
    /// built. This is the last word on it.
    override func layout() {
        super.layout()
        (layer as? CAMetalLayer)?.drawableSize = pixels
    }

    /// The link belongs to the display the view is on, so it is asked for once
    /// the view knows which that is. It follows the view to another display on
    /// its own.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // A link holds its target, and this view holds the link — so a view
        // that leaves its window and is never invalidated is a pair that keeps
        // each other alive. One window makes that invisible; the second one
        // makes it a leak.
        guard window != nil else {
            link?.invalidate()
            link = nil
            return
        }
        guard link == nil else { return }
        let link = displayLink(target: self, selector: #selector(tick))
        // Running rather than paused: a wake that arrived before there was a
        // link left the flag raised, and the first tick is what finds it.
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    @objc private func tick() {
        // One tick with nothing behind it stops the link, with no hysteresis.
        // Coming back costs a frame; what it buys is an idle app that does
        // nothing at all. cf. 05-swift-app 6.
        guard pending.lower() else {
            link?.isPaused = true
            return
        }
        guard let frame = host?.takeFrame() else { return }
        draw(frame)
    }

    /// Put one frame on the screen: the page gets what was baked for it, then
    /// the two passes go out in the order the renderer laid them in.
    private func draw(_ frame: Frame) {
        // Ahead of the encoding, and safe there: a slot appears in this list
        // on the frame it is first drawn in, so no frame still in flight can
        // be reading the region being written.
        for update in frame.atlasUpdates {
            atlas.replace(
                region: MTLRegionMake2D(
                    Int(update.x), Int(update.y), Int(metrics.width), Int(metrics.height)
                ),
                mipmapLevel: 0,
                withBytes: update.coverage,
                bytesPerRow: Int(metrics.width)
            )
        }

        // ponytail: a frame there was no drawable or no command buffer for is
        // dropped, and the screen keeps the one before it until the next wake.
        // Holding it back for the next tick is what to do instead if that is
        // ever seen — but a tick that retries is also one that cannot pause,
        // and a layer that is not on screen answers nil for as long as it is
        // not on screen. Which of those costs more is not a guess to make
        // before the milestone that has an input path to provoke it.
        guard let layer = layer as? CAMetalLayer,
            let drawable = layer.nextDrawable(),
            let commands = queue.makeCommandBuffer()
        else { return }

        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = drawable.texture
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 1)
        pass.colorAttachments[0].storeAction = .store
        guard let encoder = commands.makeRenderCommandEncoder(descriptor: pass) else { return }

        encode(frame.backgrounds.map(Instance.init), with: backgroundPipeline, into: encoder)
        encoder.setFragmentTexture(atlas, index: 0)
        encode(frame.glyphs.map(Instance.init), with: glyphPipeline, into: encoder)

        encoder.endEncoding()
        commands.present(drawable)
        commands.commit()
    }

    /// One pass: one buffer, one draw call. cf. 04-renderer R1.
    private func encode(
        _ instances: [Instance], with pipeline: MTLRenderPipelineState,
        into encoder: MTLRenderCommandEncoder
    ) {
        // A pass with nothing in it is a screen with no letters on it, which
        // is what a cleared terminal is.
        guard !instances.isEmpty else { return }
        // ponytail: two copies of every instance per pass per frame — the
        // array above, then this buffer. What the second one buys is
        // correctness without a fence, since the command buffer holds it until
        // the GPU is done with it. A ring of buffers written in place is what
        // to reach for if either copy ever shows up in a frame's cost.
        guard
            let buffer = queue.device.makeBuffer(
                bytes: instances,
                length: MemoryLayout<Instance>.stride * instances.count,
                options: .storageModeShared
            )
        else { return }

        var uniforms = uniforms
        encoder.setRenderPipelineState(pipeline)
        encoder.setVertexBuffer(buffer, offset: 0, index: 0)
        encoder.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 1)
        encoder.drawPrimitives(
            type: .triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: instances.count
        )
    }

    private static func pipeline(
        device: MTLDevice, library: MTLLibrary, vertex: String, fragment: String, blending: Bool
    ) throws -> MTLRenderPipelineState {
        let description = MTLRenderPipelineDescriptor()
        description.vertexFunction = library.makeFunction(name: vertex)
        description.fragmentFunction = library.makeFunction(name: fragment)
        let attachment = description.colorAttachments[0]!
        attachment.pixelFormat = .bgra8Unorm
        if blending {
            attachment.isBlendingEnabled = true
            attachment.sourceRGBBlendFactor = .sourceAlpha
            attachment.destinationRGBBlendFactor = .oneMinusSourceAlpha
            // The colour blends and the alpha does not: the background pass
            // left the drawable opaque, and coverage is about what shows
            // through the letter and not about what shows through the screen.
            attachment.sourceAlphaBlendFactor = .zero
            attachment.destinationAlphaBlendFactor = .one
        }
        return try device.makeRenderPipelineState(descriptor: description)
    }
}

/// What the GPU side could not be given.
struct MetalMissing: Error, CustomStringConvertible {
    let what: String

    init(_ what: String) { self.what = what }

    var description: String { "knotty could not get \(what)" }
}

/// One instance of either pass, laid out as `Shaders.metal` reads it.
///
/// Floats and not the renderer's integers: a `float4` is the one thing the two
/// languages agree the size and the alignment of without either being told.
private struct Instance {
    /// The background pass's rectangle, or the glyph pass's two origins.
    var geometry: SIMD4<Float>
    var color: SIMD4<Float>

    init(_ instance: BackgroundInstance) {
        geometry = SIMD4(
            Float(instance.x), Float(instance.y), Float(instance.width), Float(instance.height)
        )
        color = SIMD4(instance.color)
    }

    init(_ instance: GlyphInstance) {
        geometry = SIMD4(
            Float(instance.x), Float(instance.y), Float(instance.atlasX), Float(instance.atlasY)
        )
        color = SIMD4(instance.color)
    }
}

/// What both passes need and neither instance carries.
private struct Uniforms {
    var viewport: SIMD2<Float>
    var cell: SIMD2<Float>
    var atlasSide: Float
}

extension SIMD4<Float> {
    /// A resolved colour as the shaders take one. Opaque: a cell's colour is
    /// what covers what is behind it, and the glyph pass gets its coverage
    /// from the page instead.
    fileprivate init(_ color: Rgb) {
        self.init(Float(color.r) / 255, Float(color.g) / 255, Float(color.b) / 255, 1)
    }
}

/// The flag between the core's thread and the main queue.
///
/// A lock rather than an atomic, which is what would say this in one word: the
/// atomics live in `Synchronization`, and reaching for them would raise the
/// app's floor to macOS 15 for a single bit. Held for one read and one write
/// either way, and uncontended on every path there is.
private final class Pending: Sendable {
    private let raised = OSAllocatedUnfairLock(initialState: false)

    /// Raise it, answering whether this call is the one that raised it — which
    /// is the only time a block is worth posting to main.
    func raise() -> Bool {
        raised.withLock { raised in
            defer { raised = true }
            return !raised
        }
    }

    /// Lower it, answering whether it had been raised.
    func lower() -> Bool {
        raised.withLock { raised in
            defer { raised = false }
            return raised
        }
    }
}
