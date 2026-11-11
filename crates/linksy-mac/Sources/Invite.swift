// The pairing window: a QR code of the service's invite link (`create_invite`), for the
// phone's Linksy to scan. Closing it cancels the invite.

import AppKit
import CoreImage

final class InviteWindow: NSWindowController, NSWindowDelegate {
    /// Called once when the window closes, however it closes.
    var onClose: (() -> Void)?
    private let note = NSTextField(wrappingLabelWithString: "")
    private var closed = false

    init(uri: String) {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 380, height: 520), styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Pair with Linksy"
        window.isReleasedWhenClosed = false
        window.level = .floating
        window.center()
        super.init(window: window)
        window.delegate = self

        let heading = NSTextField(labelWithString: "Scan with Linksy on your phone")
        heading.font = NSFont.systemFont(ofSize: 15, weight: .semibold)
        heading.alignment = .center

        let image = NSImageView(image: InviteWindow.qrImage(uri, side: 300) ?? NSImage())
        image.imageScaling = .scaleProportionallyUpOrDown
        image.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([image.widthAnchor.constraint(equalToConstant: 300), image.heightAnchor.constraint(equalToConstant: 300)])

        note.stringValue = "Valid for 5 minutes. On another computer, run:\nlinksy pair '\(uri)'"
        note.font = NSFont.systemFont(ofSize: 11)
        note.textColor = .secondaryLabelColor
        note.alignment = .center
        note.isSelectable = true
        note.maximumNumberOfLines = 6
        note.lineBreakMode = .byCharWrapping
        note.translatesAutoresizingMaskIntoConstraints = false
        note.widthAnchor.constraint(equalToConstant: 340).isActive = true

        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancelPairing))
        cancel.keyEquivalent = "\u{1b}"

        let stack = NSStackView(views: [heading, image, note, cancel])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 14
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
        ])
        window.contentView = content
    }

    required init?(coder: NSCoder) { fatalError("not from a nib") }

    func show() {
        NSApp.activate(ignoringOtherApps: true)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    /// The phone scanned it: say so, then go.
    func paired(with name: String) {
        note.stringValue = "Paired with \(name)."
        note.textColor = .labelColor
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in self?.close() }
    }

    func failed(_ reason: String) {
        note.stringValue = "Pairing failed: \(reason)"
        note.textColor = .systemRed
    }

    @objc private func cancelPairing() {
        close()
    }

    func windowWillClose(_ notification: Notification) {
        guard !closed else { return }
        closed = true
        onClose?()
    }

    /// The link as a QR code, `side` points wide, crisp (no smoothing between modules).
    static func qrImage(_ text: String, side: CGFloat) -> NSImage? {
        guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
        filter.setValue(Data(text.utf8), forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")
        guard let code = filter.outputImage else { return nil }
        let scale = side / code.extent.width
        let scaled = code.samplingNearest().transformed(by: CGAffineTransform(scaleX: scale, y: scale))
        let rep = NSCIImageRep(ciImage: scaled)
        let image = NSImage(size: rep.size)
        image.addRepresentation(rep)
        return image
    }
}
