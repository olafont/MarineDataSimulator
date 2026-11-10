// Draws the app icon (1024×1024 PNG): a sail on the sea under a compass ring, with signal arcs.
// Usage: swift scripts/make-icon.swift assets/icon.png
import AppKit

let size: CGFloat = 1024
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon.png"

let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size), bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let ctx = NSGraphicsContext.current!.cgContext

func rgb(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat, _ a: CGFloat = 1) -> NSColor {
    NSColor(calibratedRed: r / 255, green: g / 255, blue: b / 255, alpha: a)
}

// macOS icon grid: 824 px rounded square centred in 1024, corner radius ~185.
let tile = NSRect(x: 100, y: 100, width: 824, height: 824)
let tilePath = NSBezierPath(roundedRect: tile, xRadius: 185, yRadius: 185)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -10), blur: 24, color: NSColor.black.withAlphaComponent(0.35).cgColor)
rgb(10, 40, 80).setFill()
tilePath.fill()
ctx.restoreGState()

ctx.saveGState()
tilePath.addClip()
// Sky-to-deep-water gradient.
NSGradient(colors: [rgb(16, 64, 120), rgb(8, 30, 62)])!.draw(in: tile, angle: -90)

// Sea: two wave bands at the bottom.
func wave(y: CGFloat, amp: CGFloat, color: NSColor) {
    let p = NSBezierPath()
    p.move(to: NSPoint(x: tile.minX, y: tile.minY))
    p.line(to: NSPoint(x: tile.minX, y: y))
    let steps = 64
    for i in 0...steps {
        let x = tile.minX + tile.width * CGFloat(i) / CGFloat(steps)
        p.line(to: NSPoint(x: x, y: y + amp * sin(CGFloat(i) / CGFloat(steps) * .pi * 4)))
    }
    p.line(to: NSPoint(x: tile.maxX, y: tile.minY))
    p.close()
    color.setFill()
    p.fill()
}
wave(y: 330, amp: 14, color: rgb(22, 110, 160))
wave(y: 270, amp: 12, color: rgb(14, 84, 130))

// Compass ring with tick marks, centred above the water.
let c = NSPoint(x: 512, y: 560)
let ring = NSBezierPath(ovalIn: NSRect(x: c.x - 270, y: c.y - 270, width: 540, height: 540))
ring.lineWidth = 14
rgb(255, 255, 255, 0.35).setStroke()
ring.stroke()
for i in 0..<36 {
    let a = CGFloat(i) * .pi / 18
    let r0: CGFloat = i % 9 == 0 ? 222 : 246
    let t = NSBezierPath()
    t.move(to: NSPoint(x: c.x + r0 * sin(a), y: c.y + r0 * cos(a)))
    t.line(to: NSPoint(x: c.x + 262 * sin(a), y: c.y + 262 * cos(a)))
    t.lineWidth = i % 9 == 0 ? 12 : 6
    t.lineCapStyle = .round
    t.stroke()
}

// Hull.
let hull = NSBezierPath()
hull.move(to: NSPoint(x: 330, y: 360))
hull.line(to: NSPoint(x: 700, y: 360))
hull.line(to: NSPoint(x: 640, y: 300))
hull.line(to: NSPoint(x: 385, y: 300))
hull.close()
rgb(245, 245, 245).setFill()
hull.fill()

// Mainsail and jib.
let main = NSBezierPath()
main.move(to: NSPoint(x: 500, y: 720))
main.curve(to: NSPoint(x: 360, y: 390), controlPoint1: NSPoint(x: 440, y: 640), controlPoint2: NSPoint(x: 380, y: 480))
main.line(to: NSPoint(x: 500, y: 390))
main.close()
rgb(255, 255, 255).setFill()
main.fill()
let jib = NSBezierPath()
jib.move(to: NSPoint(x: 520, y: 690))
jib.line(to: NSPoint(x: 520, y: 390))
jib.curve(to: NSPoint(x: 660, y: 390), controlPoint1: NSPoint(x: 580, y: 400), controlPoint2: NSPoint(x: 640, y: 390))
jib.curve(to: NSPoint(x: 520, y: 690), controlPoint1: NSPoint(x: 630, y: 500), controlPoint2: NSPoint(x: 580, y: 620))
rgb(220, 232, 245).setFill()
jib.fill()

// Signal arcs from the masthead (the data the simulator sends).
let mast = NSPoint(x: 510, y: 728)
for r in [40.0, 74.0, 108.0] {
    let arc = NSBezierPath()
    arc.appendArc(withCenter: mast, radius: CGFloat(r), startAngle: 20, endAngle: 70)
    arc.lineWidth = 17
    arc.lineCapStyle = .round
    rgb(255, 166, 48).setStroke()
    arc.stroke()
}
ctx.restoreGState()

NSGraphicsContext.restoreGraphicsState()
let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: out))
print("wrote \(out)")
