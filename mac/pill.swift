// The Lumbridge Voice pill: bottom-right, always on top.
// Idle it is a small round tile with the app's mark; hovering opens its buttons. While dictating,
// thinking or speaking it becomes the status line; Ask answers open it into a small card.
import AppKit
import CoreText

// MARK: - Design tokens

struct Tok {
    let dark: Bool
    /// Theme from voice.json "theme": dark (default), light, or system.
    static func current() -> Tok {
        switch (readJSON("voice.json")["theme"] as? String) ?? "dark" {
        case "light": return Tok(dark: false)
        case "system": return Tok(dark: NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) != .aqua)
        default: return Tok(dark: true)
        }
    }
    static func hex(_ v: UInt32, _ a: CGFloat = 1) -> NSColor {
        NSColor(srgbRed: CGFloat((v >> 16) & 0xff) / 255, green: CGFloat((v >> 8) & 0xff) / 255, blue: CGFloat(v & 0xff) / 255, alpha: a)
    }
    var surface1: NSColor { dark ? Tok.hex(0x111111) : Tok.hex(0xfafafa) }
    var surface2: NSColor { dark ? Tok.hex(0x191919) : Tok.hex(0xf2f2f2) }
    var surface3: NSColor { dark ? Tok.hex(0x222222) : Tok.hex(0xebebeb) }
    var border: NSColor { dark ? Tok.hex(0x2a2a2a) : Tok.hex(0xe3e3e3) }
    var borderStrong: NSColor { dark ? Tok.hex(0x3d3d3d) : Tok.hex(0xc9c9c9) }
    var fg: NSColor { dark ? Tok.hex(0xededed) : Tok.hex(0x0a0a0a) }
    var fgMuted: NSColor { dark ? Tok.hex(0xa1a1a1) : Tok.hex(0x4d4d4d) }
    var fgSubtle: NSColor { dark ? Tok.hex(0x8a8a8a) : Tok.hex(0x666666) }
    var ok: NSColor { dark ? Tok.hex(0x3fcf6f) : Tok.hex(0x13793a) }
    var warn: NSColor { dark ? Tok.hex(0xf5a524) : Tok.hex(0xa14e00) }
    var danger: NSColor { dark ? Tok.hex(0xff6166) : Tok.hex(0xc3201f) }
}

enum Fonts {
    static var registered = false
    static func register() {
        guard !registered, let dir = Bundle.main.resourceURL?.appendingPathComponent("fonts"),
              let files = try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil) else { return }
        for f in files where f.pathExtension == "ttf" { CTFontManagerRegisterFontsForURL(f as CFURL, .process, nil) }
        registered = true
    }
    static func sans(_ size: CGFloat, _ w: NSFont.Weight = .regular) -> NSFont {
        let d = NSFontDescriptor(fontAttributes: [.family: "Geist", .traits: [NSFontDescriptor.TraitKey.weight: w]])
        return NSFont(descriptor: d, size: size).flatMap { $0.familyName == "Geist" ? $0 : nil } ?? .systemFont(ofSize: size, weight: w)
    }
    static func mono(_ size: CGFloat, _ w: NSFont.Weight = .medium) -> NSFont {
        let d = NSFontDescriptor(fontAttributes: [.family: "Geist Mono", .traits: [NSFontDescriptor.TraitKey.weight: w]])
        return NSFont(descriptor: d, size: size).flatMap { $0.familyName == "Geist Mono" ? $0 : nil } ?? .monospacedSystemFont(ofSize: size, weight: w)
    }
}

// MARK: - The mark (an L on a 1024 grid), drawn in one ink

final class MarkView: NSView {
    var ink: NSColor = .white { didSet { needsDisplay = true } }
    override var isFlipped: Bool { true }
    override func draw(_ r: NSRect) {
        let s = min(bounds.width, bounds.height) / 1024
        let ox = (bounds.width - 1024 * s) / 2, oy = (bounds.height - 1024 * s) / 2
        ink.setFill()
        NSBezierPath(rect: NSRect(x: ox + 312 * s, y: oy + 212 * s, width: 132 * s, height: 600 * s)).fill()
        NSBezierPath(rect: NSRect(x: ox + 312 * s, y: oy + 680 * s, width: 400 * s, height: 132 * s)).fill()
    }
}

// MARK: - Ghost icon button (32 px, hover = one surface step)

final class PillButton: NSButton {
    var tok = Tok.current()
    var onClick: (() -> Void)?
    private var hovering = false
    init(symbol: String, tip: String) {
        super.init(frame: NSRect(x: 0, y: 0, width: 32, height: 32))
        isBordered = false
        bezelStyle = .regularSquare
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)?
            .withSymbolConfiguration(.init(pointSize: 14, weight: .regular))
        imagePosition = .imageOnly
        toolTip = tip
        setAccessibilityLabel(tip)
        wantsLayer = true
        layer?.cornerRadius = 8
        target = self; action = #selector(fire)
        translatesAutoresizingMaskIntoConstraints = false
        widthAnchor.constraint(equalToConstant: 32).isActive = true
        heightAnchor.constraint(equalToConstant: 32).isActive = true
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
        restyle()
    }
    required init?(coder: NSCoder) { fatalError() }
    @objc func fire() { onClick?() }
    func restyle() {
        contentTintColor = hovering ? tok.fg : tok.fgMuted
        layer?.backgroundColor = (hovering ? tok.surface2 : .clear).cgColor
    }
    override func mouseEntered(with e: NSEvent) { hovering = true; restyle() }
    override func mouseExited(with e: NSEvent) { hovering = false; restyle() }
}

/// Small text button for the answer card (secondary style).
final class PillTextButton: NSButton {
    var onClick: (() -> Void)?
    init(_ title: String, tok: Tok, primary: Bool = false) {
        super.init(frame: .zero)
        isBordered = false
        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.borderWidth = primary ? 0 : 1
        layer?.borderColor = tok.border.cgColor
        layer?.backgroundColor = (primary ? tok.fg : tok.surface3).cgColor
        attributedTitle = NSAttributedString(string: title, attributes: [.font: Fonts.sans(12, .medium),
                                             .foregroundColor: primary ? tok.surface1 : tok.fg])
        target = self; action = #selector(fire)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(equalToConstant: 28).isActive = true
        widthAnchor.constraint(greaterThanOrEqualToConstant: 56).isActive = true
    }
    required init?(coder: NSCoder) { fatalError() }
    @objc func fire() { onClick?() }
}

// MARK: - Container that drags the panel and reports hover

final class PillSurface: NSView {
    var onHover: ((Bool) -> Void)?
    var onDragEnd: (() -> Void)?
    var onClickBody: (() -> Void)?
    private var downAt: NSPoint?
    private var moved = false
    override init(frame: NSRect) {
        super.init(frame: frame)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }
    required init?(coder: NSCoder) { fatalError() }
    override func mouseEntered(with e: NSEvent) { onHover?(true) }
    override func mouseExited(with e: NSEvent) { onHover?(false) }
    override func mouseDown(with e: NSEvent) { downAt = NSEvent.mouseLocation; moved = false }
    override func mouseDragged(with e: NSEvent) {
        guard let w = window, let start = downAt else { return }
        let now = NSEvent.mouseLocation
        let dx = now.x - start.x, dy = now.y - start.y
        if abs(dx) + abs(dy) > 3 { moved = true }
        w.setFrameOrigin(NSPoint(x: w.frame.origin.x + dx, y: w.frame.origin.y + dy))
        downAt = now
    }
    override func mouseUp(with e: NSEvent) {
        if moved { onDragEnd?() } else { onClickBody?() }
        downAt = nil
    }
}

// MARK: - The pill

final class Pill: NSObject {
    let panel: NSPanel
    let surface = PillSurface()
    let mark = MarkView()
    let dot = NSView()
    let label = NSTextField(labelWithString: "")      // kept for callers that read the current line
    let meter = NSStackView()
    var bars: [NSView] = []
    let buttons = NSStackView()
    let card = NSStackView()                           // the Ask answer card
    var tok = Tok.current()
    var hideWork: DispatchWorkItem?
    var mode = "idle"                                  // idle | open | status | card
    var hovering = false
    var meterOn = false
    var speechText = ""
    var speechLabel: NSTextField?                       // the text being read aloud (speech line or answer card)
    var multiline = false

    // what the buttons do (set by the app)
    var onDictate: (() -> Void)?
    var onAsk: (() -> Void)?
    var onShot: (() -> Void)?
    var onRead: (() -> Void)?
    var onRecent: ((NSView) -> Void)?
    var onMore: ((NSView) -> Void)?

    let H: CGFloat = 40
    var askButton: PillButton!

    override init() {
        Fonts.register()
        panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 40, height: 40),
                        styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        super.init()
        panel.level = .statusBar
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true                         // floats, so it's the one thing with a shadow
        panel.becomesKeyOnlyIfNeeded = true
        panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        panel.isMovableByWindowBackground = false

        surface.wantsLayer = true
        surface.layer?.borderWidth = 1
        surface.layer?.masksToBounds = true
        panel.contentView = surface

        mark.translatesAutoresizingMaskIntoConstraints = false
        dot.wantsLayer = true; dot.layer?.cornerRadius = 4; dot.translatesAutoresizingMaskIntoConstraints = false
        label.font = Fonts.sans(13, .medium)
        label.lineBreakMode = .byTruncatingTail
        label.maximumNumberOfLines = 1
        label.translatesAutoresizingMaskIntoConstraints = false
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        meter.orientation = .horizontal; meter.spacing = 2; meter.alignment = .centerY
        meter.translatesAutoresizingMaskIntoConstraints = false
        for _ in 0..<5 {
            let b = NSView(); b.wantsLayer = true; b.layer?.cornerRadius = 1
            b.translatesAutoresizingMaskIntoConstraints = false
            b.widthAnchor.constraint(equalToConstant: 3).isActive = true
            let h = b.heightAnchor.constraint(equalToConstant: 4); h.isActive = true
            b.identifier = NSUserInterfaceItemIdentifier("h")
            meter.addArrangedSubview(b); bars.append(b)
        }

        buttons.orientation = .horizontal; buttons.spacing = 2
        buttons.translatesAutoresizingMaskIntoConstraints = false
        let mic = PillButton(symbol: "mic", tip: "Dictate (or tap right ⌥)")
        askButton = PillButton(symbol: "text.bubble", tip: "Ask about this screen")
        let shot = PillButton(symbol: "camera.viewfinder", tip: "Screenshot a region")
        let read = PillButton(symbol: "speaker.wave.2", tip: "Read the selection aloud (⌃⌥S)")
        let recent = PillButton(symbol: "clock.arrow.circlepath", tip: "Recent dictations")
        let more = PillButton(symbol: "ellipsis", tip: "More")
        mic.onClick = { [weak self] in self?.onDictate?() }
        askButton.onClick = { [weak self] in self?.onAsk?() }
        shot.onClick = { [weak self] in self?.onShot?() }
        read.onClick = { [weak self] in self?.onRead?() }
        recent.onClick = { [weak self] in if let s = self { s.onRecent?(recent) } }
        more.onClick = { [weak self] in if let s = self { s.onMore?(more) } }
        for b in [mic, askButton!, shot, read, recent, more] { buttons.addArrangedSubview(b) }

        card.orientation = .vertical; card.alignment = .leading; card.spacing = 10
        card.edgeInsets = NSEdgeInsets(top: 14, left: 16, bottom: 12, right: 16)
        card.translatesAutoresizingMaskIntoConstraints = false

        for v in [mark, dot, label, meter, buttons, card] as [NSView] { surface.addSubview(v) }
        surface.onHover = { [weak self] on in self?.hover(on) }
        surface.onDragEnd = { [weak self] in self?.saveOrigin() }
        surface.onClickBody = { [weak self] in
            guard let s = self else { return }
            if s.mode == "idle" { s.layout("open") }
        }
        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in self?.place() }
        DistributedNotificationCenter.default().addObserver(forName: Notification.Name("AppleInterfaceThemeChangedNotification"), object: nil, queue: .main) { [weak self] _ in
            self?.restyle()
        }
        restyle()
        layout("idle")
        panel.orderFrontRegardless()
    }

    // MARK: style and layout

    func restyle() {
        tok = Tok.current()
        surface.layer?.backgroundColor = tok.surface1.cgColor
        surface.layer?.borderColor = tok.border.cgColor
        mark.ink = tok.fg
        label.textColor = tok.fg
        for b in bars { b.layer?.backgroundColor = tok.fgMuted.cgColor }
        for case let b as PillButton in buttons.arrangedSubviews { b.tok = tok; b.restyle() }
    }

    private var constraintsNow: [NSLayoutConstraint] = []

    /// Lays out one of the modes and resizes the panel around the bottom-right anchor.
    func layout(_ m: String, width: CGFloat? = nil, height: CGFloat? = nil) {
        mode = m
        NSLayoutConstraint.deactivate(constraintsNow)
        var c: [NSLayoutConstraint] = []
        let show: [NSView]
        switch m {
        case "idle":
            show = [mark]
            c += [mark.centerXAnchor.constraint(equalTo: surface.centerXAnchor), mark.centerYAnchor.constraint(equalTo: surface.centerYAnchor),
                  mark.widthAnchor.constraint(equalToConstant: 20), mark.heightAnchor.constraint(equalToConstant: 20)]
        case "open":
            show = [mark, buttons]
            c += [mark.leadingAnchor.constraint(equalTo: surface.leadingAnchor, constant: 10), mark.centerYAnchor.constraint(equalTo: surface.centerYAnchor),
                  mark.widthAnchor.constraint(equalToConstant: 20), mark.heightAnchor.constraint(equalToConstant: 20),
                  buttons.leadingAnchor.constraint(equalTo: mark.trailingAnchor, constant: 8), buttons.centerYAnchor.constraint(equalTo: surface.centerYAnchor)]
        case "card", "speech":
            show = [card]
            c += [card.leadingAnchor.constraint(equalTo: surface.leadingAnchor), card.trailingAnchor.constraint(equalTo: surface.trailingAnchor),
                  card.topAnchor.constraint(equalTo: surface.topAnchor), card.bottomAnchor.constraint(equalTo: surface.bottomAnchor)]
        default: // status
            show = [dot, label, meter]
            c += [dot.leadingAnchor.constraint(equalTo: surface.leadingAnchor, constant: 16),
                  multiline ? dot.topAnchor.constraint(equalTo: surface.topAnchor, constant: 16) : dot.centerYAnchor.constraint(equalTo: surface.centerYAnchor),
                  dot.widthAnchor.constraint(equalToConstant: 8), dot.heightAnchor.constraint(equalToConstant: 8),
                  label.leadingAnchor.constraint(equalTo: dot.trailingAnchor, constant: 10),
                  multiline ? label.topAnchor.constraint(equalTo: surface.topAnchor, constant: 11) : label.centerYAnchor.constraint(equalTo: surface.centerYAnchor),
                  dot.widthAnchor.constraint(equalToConstant: 8)]
            if multiline {
                c += [label.trailingAnchor.constraint(equalTo: surface.trailingAnchor, constant: -16)]
            } else {
                c += [meter.leadingAnchor.constraint(equalTo: label.trailingAnchor, constant: 10), meter.centerYAnchor.constraint(equalTo: surface.centerYAnchor),
                      label.trailingAnchor.constraint(lessThanOrEqualTo: surface.trailingAnchor, constant: -16),
                      meter.trailingAnchor.constraint(lessThanOrEqualTo: surface.trailingAnchor, constant: -16)]
            }
        }
        for v in [mark, dot, label, meter, buttons, card] as [NSView] { v.isHidden = !show.contains(v) }
        if m == "status" && !meterOn { meter.isHidden = true }
        NSLayoutConstraint.activate(c); constraintsNow = c

        let w: CGFloat, h: CGFloat
        switch m {
        case "idle": w = H; h = H
        case "open": w = 10 + 20 + 8 + CGFloat(buttons.arrangedSubviews.count) * 34 + 6; h = H
        case "card", "speech": w = width ?? 380; h = height ?? 120
        default:
            if multiline {
                w = 440
                let th = label.sizeThatFits(NSSize(width: 440 - 16 - 8 - 10 - 16, height: 400)).height
                h = max(H, ceil(th) + 22)
            } else {
                let tw = (label.stringValue as NSString).size(withAttributes: [.font: label.font!]).width
                w = min(480, max(160, 16 + 8 + 10 + tw + (meterOn ? 10 + 23 : 0) + 18)); h = H
            }
        }
        surface.layer?.cornerRadius = (m == "card" || m == "speech" || (m == "status" && multiline)) ? 12 : h / 2
        resize(w, h)
    }

    func anchor() -> NSPoint {
        // bottom-right corner of the pill, remembered per display as an offset from its bottom-right
        let scr = NSScreen.main ?? NSScreen.screens[0]
        let f = scr.visibleFrame
        let o = readJSON("voice.json")["pill_offset"] as? [Double]
        let dx = CGFloat(o?.first ?? 16), dy = CGFloat(o?.last ?? 16)
        return NSPoint(x: f.maxX - dx, y: f.minY + dy)
    }

    func resize(_ w: CGFloat, _ h: CGFloat) {
        let a = anchor()
        panel.setFrame(NSRect(x: a.x - w, y: a.y, width: w, height: h), display: true, animate: false)
    }

    func place() { layout(mode) }

    func saveOrigin() {
        let f = panel.frame, scr = (panel.screen ?? NSScreen.main ?? NSScreen.screens[0]).visibleFrame
        var o = readJSON("voice.json")
        o["pill_offset"] = [Double(max(0, scr.maxX - f.maxX)), Double(max(0, f.minY - scr.minY))]
        writeJSON("voice.json", o)
    }

    func hover(_ on: Bool) {
        hovering = on
        if on && mode == "idle" { layout("open") }
        if !on && mode == "open" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { if !self.hovering && self.mode == "open" { self.layout("idle") } }
        }
    }

    // MARK: API used by the app (same calls as the old HUD)

    /// kind: rec | lock | busy | speak | update | ok | warn
    func show(_ text: String, _ kind: String, hideAfter: Double? = nil) {
        DispatchQueue.main.async {
            if (self.mode == "card" || self.mode == "speech") && kind != "rec" && kind != "lock" && kind != "busy" { return }  // don't clobber what the user is reading
            self.hideWork?.cancel()
            let colors: [String: NSColor] = ["rec": self.tok.ok, "lock": self.tok.ok, "busy": self.tok.fgMuted, "speak": self.tok.fg,
                                             "update": self.tok.fg, "ok": self.tok.ok, "warn": self.tok.warn]
            self.dot.layer?.backgroundColor = (colors[kind] ?? self.tok.fgMuted).cgColor
            self.label.stringValue = text
            self.meterOn = kind == "rec" || kind == "lock"
            let tw = (text as NSString).size(withAttributes: [.font: self.label.font!]).width
            self.multiline = tw > 420
            self.label.maximumNumberOfLines = self.multiline ? 3 : 1
            self.label.lineBreakMode = self.multiline ? .byWordWrapping : .byTruncatingTail
            self.label.preferredMaxLayoutWidth = self.multiline ? 440 - 16 - 8 - 10 - 16 : 0
            self.label.cell?.wraps = self.multiline
            self.label.cell?.truncatesLastVisibleLine = true
            self.layout("status")
            self.panel.orderFrontRegardless()
            if let h = hideAfter {
                let w = DispatchWorkItem { self.layout("idle") }
                self.hideWork = w
                DispatchQueue.main.asyncAfter(deadline: .now() + h, execute: w)
            }
        }
    }

    func hide() { DispatchQueue.main.async { if self.mode != "card" { self.hideWork?.cancel(); self.layout("idle") } } }

    // MARK: read-along

    /// Full text while it's spoken: said words in ink, the current word marked, the rest muted. Never truncated.
    func showSpeech(_ text: String, title: String) {
        DispatchQueue.main.async {
            self.hideWork?.cancel()
            self.card.arrangedSubviews.forEach { $0.removeFromSuperview() }
            let width: CGFloat = 440
            let head = NSTextField(labelWithString: "")
            head.attributedStringValue = NSAttributedString(string: title.uppercased(),
                attributes: [.font: Fonts.mono(11), .kern: 0.9, .foregroundColor: self.tok.fgSubtle])
            let body = NSTextField(wrappingLabelWithString: text)
            body.font = Fonts.sans(14)
            body.maximumNumberOfLines = 0
            body.preferredMaxLayoutWidth = width - 32
            body.isSelectable = true
            self.speechText = text
            self.speechLabel = body
            self.paint(-1)
            for v in [head, body] { self.card.addArrangedSubview(v) }
            body.widthAnchor.constraint(equalToConstant: width - 32).isActive = true
            let bh = body.sizeThatFits(NSSize(width: width - 32, height: 4000)).height
            let scr = (NSScreen.main ?? NSScreen.screens[0]).visibleFrame
            let h = min(scr.height * 0.6, 14 + 14 + 10 + ceil(bh) + 14)
            self.layout("speech", width: width, height: h)
            self.panel.orderFrontRegardless()
        }
    }

    func speechProgress(_ i: Int) { DispatchQueue.main.async { self.paint(i) } }

    /// Lingers a moment after the voice stops so the last line can be read, then folds back.
    func speechDone() {
        DispatchQueue.main.async {
            self.paint(Int.max)
            guard self.mode == "speech" else { return }
            let w = DispatchWorkItem { if self.mode == "speech" { self.layout("idle") } }
            self.hideWork = w
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.5, execute: w)
        }
    }

    private func paint(_ upTo: Int) {
        guard let label = speechLabel else { return }
        let text = speechText as NSString
        let n = text.length
        let a = NSMutableAttributedString(string: speechText, attributes: [.font: Fonts.sans(14), .foregroundColor: tok.fgSubtle])
        let i = max(0, min(n, upTo))
        if i > 0 { a.addAttribute(.foregroundColor, value: tok.fg, range: NSRange(location: 0, length: i)) }
        if upTo >= 0 && upTo < n {
            // the word under the voice right now
            var s = i, e = i
            while s > 0, !CharacterSet.whitespacesAndNewlines.contains(UnicodeScalar(text.character(at: s - 1)) ?? " ") { s -= 1 }
            while e < n, !CharacterSet.whitespacesAndNewlines.contains(UnicodeScalar(text.character(at: e)) ?? " ") { e += 1 }
            if e > s {
                a.addAttributes([.foregroundColor: tok.fg, .backgroundColor: tok.dark ? Tok.hex(0x3d3d3d) : Tok.hex(0xdcdcdc)],
                                range: NSRange(location: s, length: e - s))
            }
        }
        label.attributedStringValue = a
    }

    func level(_ v: Float) {
        DispatchQueue.main.async {
            guard self.meterOn else { return }
            let shape: [CGFloat] = [0.55, 0.85, 1, 0.8, 0.6]
            for (i, b) in self.bars.enumerated() {
                let jitter = CGFloat.random(in: 0.75...1.0)
                let h = 4 + CGFloat(min(1, max(0, v))) * 14 * shape[i] * jitter
                b.constraints.first { $0.firstAttribute == .height }?.constant = h
            }
        }
    }

    /// The Ask answer card: a mono label, the question, the answer, and actions.
    func showAnswer(question: String, answer: String, actions: [(String, Bool, () -> Void)]) {
        DispatchQueue.main.async {
            self.hideWork?.cancel()
            self.card.arrangedSubviews.forEach { $0.removeFromSuperview() }
            let width: CGFloat = 400
            let head = NSTextField(labelWithString: "ASK")
            head.font = Fonts.mono(11); head.textColor = self.tok.fgSubtle
            head.attributedStringValue = NSAttributedString(string: "ASK", attributes: [.font: Fonts.mono(11), .kern: 0.9, .foregroundColor: self.tok.fgSubtle])
            let q = NSTextField(wrappingLabelWithString: question)
            q.font = Fonts.sans(12); q.textColor = self.tok.fgMuted; q.maximumNumberOfLines = 2
            q.preferredMaxLayoutWidth = width - 32
            let a = NSTextField(wrappingLabelWithString: answer)
            a.font = Fonts.sans(14); a.textColor = self.tok.fg; a.maximumNumberOfLines = 0
            a.preferredMaxLayoutWidth = width - 32
            a.isSelectable = true
            let row = NSStackView(); row.orientation = .horizontal; row.spacing = 8
            for (title, primary, act) in actions {
                let b = PillTextButton(title, tok: self.tok, primary: primary); b.onClick = act; row.addArrangedSubview(b)
            }
            for v in [head, q, a, row] { self.card.addArrangedSubview(v) }
            self.speechText = answer; self.speechLabel = a; self.paint(-1)
            for v in [q, a] { v.widthAnchor.constraint(equalToConstant: width - 32).isActive = true }
            let qh = q.sizeThatFits(NSSize(width: width - 32, height: 1000)).height
            let ah = a.sizeThatFits(NSSize(width: width - 32, height: 1000)).height
            let scr = (NSScreen.main ?? NSScreen.screens[0]).visibleFrame
            let h = min(scr.height * 0.7, 14 + 14 + 10 + qh + 10 + ceil(ah) + 10 + 28 + 12)
            self.layout("card", width: width, height: h)
            self.panel.orderFrontRegardless()
        }
    }

    func closeCard() { DispatchQueue.main.async { self.speechLabel = nil; self.card.arrangedSubviews.forEach { $0.removeFromSuperview() }; self.layout("idle") } }

    func setAsking(_ on: Bool) {
        DispatchQueue.main.async { self.askButton.contentTintColor = on ? self.tok.ok : self.tok.fgMuted }
    }
}
