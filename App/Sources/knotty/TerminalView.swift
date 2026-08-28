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

    /// The composition an input method has open, or nil when there is none.
    ///
    /// The one piece of typed text this view holds on to, and it is held
    /// precisely so that it does not go down: text still being made is not in
    /// the terminal, and a half-made syllable in the grid could not be taken
    /// back. cf. 05-swift-app 7.
    private var marked: NSAttributedString?
    /// What an input method committed during the `keyDown` being handled.
    private var committedText: String?
    /// Whether a `keyDown` is the reason an input method is calling back.
    private var handlingKeyDown = false
    /// Where a composition is drawn: over the cursor, above both passes, and
    /// off the cell grid — which is why it is a view and not a third pass.
    private let preedit = NSTextField(labelWithString: "")
    private let preeditFont: NSFont
    /// One cell in points, which is what places anything AppKit lays out.
    private let cellSize: NSSize
    /// What watches for the window to stop being the one typed into.
    private var resigning: (any NSObjectProtocol)?

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
        cellSize = NSSize(
            width: Double(metrics.width) / scale, height: Double(metrics.height) / scale
        )
        // The same face `CellMetrics` measured the grid from — the system's
        // fixed-pitch font, which is the one this milestone draws with — asked
        // for in points, because the overlay is AppKit's to lay out and not
        // the grid's. cf. 05-swift-app 10 for when the two stop being a
        // constant and start being configured together.
        let pointSize = metrics.fontPixelSize / scale
        preeditFont =
            NSFont.userFixedPitchFont(ofSize: pointSize)
            ?? .monospacedSystemFont(ofSize: pointSize, weight: .regular)

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

        // ponytail: the composition is drawn white on black, which is what
        // this milestone's terminal is. The theme it should take these from
        // arrives with the configuration pipeline in M4. cf. 05-swift-app 10.
        preedit.drawsBackground = true
        preedit.backgroundColor = .black
        preedit.isHidden = true
        addSubview(preedit)

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
        guard let window else {
            resigning.map(NotificationCenter.default.removeObserver)
            resigning = nil
            link?.invalidate()
            link = nil
            return
        }
        // The window and not the responder chain, because this window has one
        // view and nothing ever takes first responder from it: what really
        // ends a composition here is the window ceasing to be the one being
        // typed into. cf. 07-definition-of-done C.
        resigning = resigning
            ?? NotificationCenter.default.addObserver(
                forName: NSWindow.didResignKeyNotification, object: window, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.endComposition() }
            }
        guard link == nil else { return }
        let link = displayLink(target: self, selector: #selector(tick))
        // Running rather than paused: a wake that arrived before there was a
        // link left the flag raised, and the first tick is what finds it.
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    /// What makes a window's keys arrive here at all.
    override var acceptsFirstResponder: Bool { true }

    /// What the user typed, on its way to the child — by way of the input
    /// method, which gets the first say. cf. 05-swift-app 7.
    ///
    /// What the method did with the key is read off what it called back and
    /// not off its answer: a Roman layout reports every plain letter consumed
    /// and hands it straight back through ``insertText(_:replacementRange:)``,
    /// so an answer of "handled" says nothing about whose key it was. Whether
    /// a composition was open across it does.
    override func keyDown(with event: NSEvent) {
        let wasComposing = marked != nil
        committedText = nil
        handlingKeyDown = true
        interpretKeyEvents([event])
        handlingKeyDown = false

        // A composition the key was open across owns it. What the method
        // finished making goes down as the text it already is — it stands at
        // no place on the keyboard, so there is nothing for the core to encode
        // it from — and the key itself is spent on ending the composition.
        if wasComposing {
            // Nothing committed is a composition still being made — a jamo
            // deleted, a candidate stepped through — and none of that is the
            // terminal's.
            guard let committedText else { return }
            if Self.isText(committedText) {
                host?.write(committedText)
            }
            if marked == nil, Self.movesTheCursor(event) {
                host?.send(Self.key(of: event, composing: false))
            }
            return
        }
        // No composition was open across it. Whatever the method made of it is
        // then only the layout's answer to this very key, and the key is what
        // carries that down — to where the modes that decide its bytes are
        // read. A key that opened a composition goes down too, carrying the
        // flag the engine reads to leave it alone. cf. adr/0017.
        host?.send(Self.key(of: event, composing: marked != nil))
    }

    /// One `NSEvent` as the core takes a key: undecided.
    ///
    /// Which bytes it comes to depends on modes the terminal holds, and
    /// nothing here reads one. What this adds is only what AppKit knows and
    /// the core cannot — where on the keyboard the key sits, what was held
    /// with it, what the layout made of it, and whether a composition is open.
    /// cf. adr/0017.
    private static func key(of event: NSEvent, composing: Bool) -> KeyEvent {
        KeyEvent(
            macOSKeyCode: event.keyCode,
            // A key held down is the platform repeating it, and the core has
            // an action of its own for that.
            action: event.isARepeat ? .repeat : .press,
            mods: Modifiers(event.modifierFlags),
            consumedMods: consumedModifiers(of: event),
            text: event.characters ?? "",
            composing: composing
        )
    }

    /// Whether what an input method committed is text at all.
    ///
    /// A method composing across `⌃H` hands the control character straight
    /// back rather than acting on it, and a terminal reading that as text
    /// would see a backspace nobody typed. A lone character is the whole of
    /// the case: what made one is a key still on its way down, and the core
    /// derives the control character from that key — which is the half of the
    /// rule ``KnottySession/KeyEvent`` keeps its own text by that applies
    /// here, the other half being about codepoints only AppKit puts on a key.
    private static func isText(_ committed: String) -> Bool {
        guard committed.unicodeScalars.count == 1,
            let scalar = committed.unicodeScalars.first
        else { return !committed.isEmpty }
        return scalar.value >= 0x20 && scalar.value != 0x7f
    }

    /// Which keys a finished composition still hands on to the terminal.
    ///
    /// An input method that commits on a cursor key does nothing further with
    /// it, so the movement is the terminal's to make — and only then, since a
    /// cursor key the method spent stepping through its own candidates commits
    /// nothing. Everything else that ends a composition — Enter, Escape — is
    /// spent on ending it, and a second press is what carries it down. Where
    /// exactly this line falls is what the manual pass over DoD C is for.
    private static func movesTheCursor(_ event: NSEvent) -> Bool {
        switch event.specialKey {
        case .upArrow, .downArrow, .leftArrow, .rightArrow: true
        default: false
        }
    }

    /// What an input method reaches the responder chain with for anything it
    /// did not turn into text — a cursor key, a deletion. Nothing here acts on
    /// one: the key itself is still on its way to the terminal, which is where
    /// every one of these commands belongs. Swallowing it is what keeps AppKit
    /// from beeping at a command no responder up the chain implements.
    nonisolated override func doCommand(by selector: Selector) {}

    /// Focus leaving takes an unfinished composition with it.
    ///
    /// Nothing of one was ever in the grid — that is what makes it droppable
    /// rather than something to unpick — but an overlay left standing over a
    /// window no longer being typed into is the one way it could look as
    /// though it had been. cf. 07-definition-of-done C.
    ///
    /// The window losing key is the path that really runs here; this is the
    /// one that will, once a window holds more than the terminal.
    override func resignFirstResponder() -> Bool {
        endComposition()
        return super.resignFirstResponder()
    }

    /// Drop an unfinished composition, telling the input method so.
    private func endComposition() {
        guard marked != nil else { return }
        inputContext?.discardMarkedText()
        unmark()
    }

    /// Where the cursor's cell sits in the view.
    ///
    /// The renderer places from the top left in device pixels and a view
    /// counts from the bottom left in points, which is the whole of this. The
    /// cell itself comes off the snapshot: this view keeps no cursor of its
    /// own. cf. 05-swift-app 7.
    private func cursorRect() -> NSRect? {
        guard let cell = host?.cursorCell else { return nil }
        return NSRect(
            x: Double(cell.column) * cellSize.width,
            y: bounds.height - Double(cell.row + 1) * cellSize.height,
            width: cellSize.width,
            height: cellSize.height
        )
    }

    /// Show the composition, in the one face the grid is drawn in.
    private func show(_ text: NSAttributedString) {
        let shown = NSMutableAttributedString(attributedString: text)
        shown.addAttributes(
            [.font: preeditFont, .foregroundColor: NSColor.white],
            range: NSRange(location: 0, length: shown.length)
        )
        preedit.attributedStringValue = shown
        preedit.sizeToFit()
        placeComposition()
    }

    /// Put the composition where the cursor now is, and tell the input method
    /// that what it last asked about has moved.
    ///
    /// Called on every frame an open composition sees: output nobody typed
    /// moves the cursor out from under it, and macOS holds on to the rectangle
    /// ``firstRect(forCharacterRange:actualRange:)`` answered with until it is
    /// told that answer went stale.
    private func placeComposition() {
        // No cursor is nowhere to put it. The terminal hides one, and a
        // composition drawn where the last one stood points at a cell that no
        // longer means anything.
        guard marked != nil, let cell = cursorRect() else {
            preedit.isHidden = true
            return
        }
        preedit.setFrameOrigin(cell.origin)
        preedit.isHidden = false
        inputContext?.invalidateCharacterCoordinates()
    }

    /// Take the composition down. What was never committed was never in the
    /// terminal, so there is nothing to undo — only an overlay to stop showing.
    private func unmark() {
        marked = nil
        preedit.isHidden = true
    }

    /// A string an input method handed over, which it may do either way. A
    /// bare one is underlined here, so that text in the middle of being made
    /// never looks like text that already is.
    private static func attributed(_ string: Any) -> NSAttributedString {
        if let attributed = string as? NSAttributedString { return attributed }
        return NSAttributedString(
            string: string as? String ?? "",
            attributes: [.underlineStyle: NSUnderlineStyle.single.rawValue]
        )
    }

    /// Which of what was held the layout already spent on the characters.
    ///
    /// Option is the one macOS spends: `⌥A` is `å` on a US layout, and a
    /// terminal encoding Meta on top of that would be the same modifier
    /// counted twice. What tells the two apart is whether the characters
    /// differ from the ones the key makes without it —
    /// `charactersIgnoringModifiers` drops every modifier but shift, so an
    /// arrow or a plain letter answers the same string either way.
    ///
    /// Control and Command are asked after because they change the characters
    /// too, and what they changed is not Option's to be credited with: `⌃⌥A`
    /// is the control character Control made, and calling Option spent on it
    /// is what would take the Meta prefix back off.
    private static func consumedModifiers(of event: NSEvent) -> Modifiers {
        guard event.modifierFlags.contains(.option),
            event.modifierFlags.isDisjoint(with: [.control, .command]),
            event.characters != event.charactersIgnoringModifiers
        else { return [] }
        return .alt
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
        placeComposition()
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

/// The input method's side of the view. cf. 05-swift-app 7.
///
/// The document an input method is shown is the composition and nothing else.
/// What is already on the screen is the child's text, not this view's to offer
/// back for replacement or to be asked about — so the ranges are the marked
/// text's own, the selection is empty, and there is no substring to hand out.
///
/// Every member is `nonisolated`, and the ones that touch the view assume the
/// main actor rather than being declared on it. AppKit calls all of these on
/// the main thread and 05-swift-app 5 already says so; what the annotation is
/// really about is the SDK, which declares this protocol isolated on macOS 26
/// and unisolated on the one CI builds against. A witness that is isolated to
/// nothing satisfies it either way, and the isolated-conformance syntax that
/// would say it once is newer than the compiler on that runner.
extension TerminalView: NSTextInputClient {
    /// Text an input method finished making.
    ///
    /// Inside a `keyDown` it is that key's answer and ``keyDown(with:)``
    /// decides what becomes of it. Outside one there is no key at all — the
    /// emoji palette and the character viewer insert straight into the client —
    /// so it goes down where it arrives, by the path a composition takes.
    nonisolated func insertText(_ string: Any, replacementRange: NSRange) {
        // AppKit hands this over on the main thread and keeps no reference of
        // its own to it, which is what makes the hop below sound and what
        // region isolation has no way to see. cf. 05-swift-app 5.
        nonisolated(unsafe) let string = string
        MainActor.assumeIsolated {
            let text = Self.attributed(string).string
            unmark()
            // Appended rather than assigned: one key can be committed in more
            // than one call, and the second overwriting the first would drop
            // what the method already handed over.
            if handlingKeyDown {
                committedText = (committedText ?? "") + text
            } else if !text.isEmpty {
                host?.write(text)
            }
        }
    }

    nonisolated func setMarkedText(
        _ string: Any, selectedRange: NSRange, replacementRange: NSRange
    ) {
        nonisolated(unsafe) let string = string
        MainActor.assumeIsolated {
            let text = Self.attributed(string)
            // An input method ends a composition by marking it empty as
            // readily as by unmarking it, and an empty overlay left on screen
            // is a black rectangle over a cell.
            guard text.length > 0 else {
                unmark()
                return
            }
            marked = text
            show(text)
        }
    }

    nonisolated func unmarkText() {
        MainActor.assumeIsolated { unmark() }
    }

    nonisolated func hasMarkedText() -> Bool {
        MainActor.assumeIsolated { marked != nil }
    }

    nonisolated func markedRange() -> NSRange {
        MainActor.assumeIsolated {
            guard let marked else { return NSRange(location: NSNotFound, length: 0) }
            return NSRange(location: 0, length: marked.length)
        }
    }

    /// Empty, at the start: the caret a composition is being made at is the
    /// terminal's cursor, and there is no selection of this view's for an
    /// input method to be replacing.
    nonisolated func selectedRange() -> NSRange {
        NSRange(location: 0, length: 0)
    }

    /// Nothing. An input method asking to read back what surrounds the cursor
    /// is asking about a document that is not here — the grid is the child's.
    nonisolated func attributedSubstring(
        forProposedRange range: NSRange, actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        nil
    }

    /// What marked text may be styled with, which is what an input method
    /// checks before sending any of it. The clause segments a Japanese
    /// conversion underlines apart from one another are the third.
    nonisolated func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        [.underlineStyle, .underlineColor, .markedClauseSegment]
    }

    /// Where the candidate window goes: the cursor's cell, in screen
    /// coordinates. Taken from the snapshot's cursor and not from a count of
    /// this view's own. cf. 05-swift-app 7.
    nonisolated func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?)
        -> NSRect
    {
        MainActor.assumeIsolated {
            guard let cell = cursorRect(), let window else { return .zero }
            return window.convertToScreen(convert(cell, to: nil))
        }
    }

    /// No cell answers for a character: the composition is drawn over the grid
    /// rather than in it, and nothing an input method could point at is text
    /// that it owns.
    nonisolated func characterIndex(for point: NSPoint) -> Int {
        NSNotFound
    }
}

extension Modifiers {
    /// What AppKit says was held, as the core counts it.
    ///
    /// Two of AppKit's bits are dropped rather than translated: `.function` is
    /// a key of its own here and travels as one, and `.numericPad` says where
    /// the key was rather than what was held down with it.
    fileprivate init(_ flags: NSEvent.ModifierFlags) {
        self = []
        if flags.contains(.shift) { insert(.shift) }
        if flags.contains(.control) { insert(.ctrl) }
        if flags.contains(.option) { insert(.alt) }
        if flags.contains(.command) { insert(.super) }
        if flags.contains(.capsLock) { insert(.capsLock) }
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
