// Draws the app icon (1024×1024 PNG): a white radar (range rings, cross, sweep and two
// targets) on a teal-to-navy gradient. On the macOS icon grid: 824 px rounded square, corner
// radius 185, soft shadow.
// Usage: swift scripts/make-icon.swift assets/icon.png
import AppKit

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "assets/icon.png"
func rgb(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat) -> CGColor {
    NSColor(calibratedRed: r / 255, green: g / 255, blue: b / 255, alpha: 1).cgColor
}

let size: CGFloat = 1024
let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size), bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let ctx = NSGraphicsContext.current!.cgContext
ctx.interpolationQuality = .high

let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let tilePath = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)

// Shadow under the tile.
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -10), blur: 24, color: NSColor.black.withAlphaComponent(0.35).cgColor)
ctx.addPath(tilePath)
ctx.setFillColor(NSColor(calibratedRed: 10 / 255, green: 50 / 255, blue: 90 / 255, alpha: 1).cgColor)
ctx.fillPath()
ctx.restoreGState()

// Background, clipped to the tile: teal at the top to navy at the bottom.
ctx.saveGState()
ctx.addPath(tilePath)
ctx.clip()
let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(),
                          colors: [rgb(44, 128, 164), rgb(14, 72, 112), rgb(6, 44, 82)] as CFArray,
                          locations: [0, 0.55, 1])!
ctx.drawLinearGradient(gradient, start: CGPoint(x: 0, y: tile.maxY), end: CGPoint(x: 0, y: tile.minY), options: [])

// The radar, in white.
let c = CGPoint(x: tile.midX, y: tile.midY)
let r: CGFloat = 330
func white(_ a: CGFloat) -> CGColor { NSColor(white: 1, alpha: a).cgColor }

// Sweep: a wedge that fades from the beam backwards.
let beam: CGFloat = 50 * .pi / 180          // beam bearing, counter-clockwise from east
let trail: CGFloat = 70 * .pi / 180
let steps = 70
for i in 0..<steps {
    let a0 = beam - trail * CGFloat(i + 1) / CGFloat(steps)
    let a1 = beam - trail * CGFloat(i) / CGFloat(steps)
    ctx.move(to: c)
    ctx.addArc(center: c, radius: r, startAngle: a0, endAngle: a1, clockwise: false)
    ctx.closePath()
    ctx.setFillColor(white(0.30 * pow(1 - CGFloat(i) / CGFloat(steps), 1.6)))
    ctx.fillPath()
}

// Range rings and cross.
ctx.setStrokeColor(white(0.95))
ctx.setLineWidth(16)
ctx.strokeEllipse(in: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r))
ctx.setStrokeColor(white(0.55))
ctx.setLineWidth(8)
for f in [2.0 / 3.0, 1.0 / 3.0] as [CGFloat] {
    ctx.strokeEllipse(in: CGRect(x: c.x - r * f, y: c.y - r * f, width: 2 * r * f, height: 2 * r * f))
}
ctx.setLineWidth(6)
ctx.setStrokeColor(white(0.40))
ctx.move(to: CGPoint(x: c.x - r, y: c.y)); ctx.addLine(to: CGPoint(x: c.x + r, y: c.y))
ctx.move(to: CGPoint(x: c.x, y: c.y - r)); ctx.addLine(to: CGPoint(x: c.x, y: c.y + r))
ctx.strokePath()

// Beam line.
ctx.setStrokeColor(white(1))
ctx.setLineWidth(12)
ctx.setLineCap(.round)
ctx.move(to: c)
ctx.addLine(to: CGPoint(x: c.x + r * cos(beam), y: c.y + r * sin(beam)))
ctx.strokePath()

// Targets: one just swept (bright), one older (dimmer), and the centre.
func dot(_ angle: CGFloat, _ range: CGFloat, _ radius: CGFloat, _ alpha: CGFloat) {
    let p = CGPoint(x: c.x + range * cos(angle * .pi / 180), y: c.y + range * sin(angle * .pi / 180))
    ctx.setFillColor(white(alpha))
    ctx.fillEllipse(in: CGRect(x: p.x - radius, y: p.y - radius, width: 2 * radius, height: 2 * radius))
}
dot(35, 0.62 * r, 26, 1)
dot(200, 0.80 * r, 20, 0.6)
dot(0, 0, 22, 1)
ctx.restoreGState()

NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
print("wrote \(out)")
