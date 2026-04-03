import SwiftUI

// ── CellRenderer ──────────────────────────────────────────────────────────────
//
// Stateless helpers that turn a decoded [VisualizerCell] array into SwiftUI
// Canvas draw calls.
//
// Grid layout
// ───────────
// The tvOS screen is 1920×1080 points (regardless of whether the display is
// 4K or 1080p — the system scales automatically).
//
//   cols = 80,  rows = 45
//   cellW = 1920 / 80 = 24 pt,  cellH = 1080 / 45 = 24 pt
//
// Rendering strategy
// ──────────────────
// Block characters (█ ▓ ▒ ░ and half-blocks ▀ ▄ ▌ ▐) are drawn as filled
// rectangles via ctx.fill(Path(rect)).  This eliminates the gaps that SwiftUI
// Text rendering introduces through internal line-height and padding.
//
// All other characters (letters, digits, symbols, box-drawing, etc.) are drawn
// with ctx.draw(Text(…)) using a monospaced font sized at 95% of the cell to
// minimise inter-cell gaps while avoiding clipping.
//
// Performance
// ───────────
// At 80×45 = 3 600 cells, drawing each cell individually is the
// straightforward approach and comfortably within Apple TV's GPU budget at
// 45 fps.  If profiling ever shows a bottleneck, switch to a Metal-backed
// layer that uploads a texture atlas.

enum CellRenderer {

    // MARK: - Grid constants

    /// Character grid width used by the Rust core.
    static let cols = 80
    /// Character grid height used by the Rust core.
    static let rows = 45

    // MARK: - Block character tables

    /// Full-block characters drawn as filled rectangles covering the entire cell.
    /// Mapped to opacity: █ = 1.0, ▓ = 0.75, ▒ = 0.50, ░ = 0.25.
    private static let fullBlockOpacity: [Character: Double] = [
        "█": 1.0,
        "▓": 0.75,
        "▒": 0.50,
        "░": 0.25,
    ]

    /// Half-block characters drawn as filled rectangles covering part of the cell.
    /// Values are (xFrac, yFrac, wFrac, hFrac) relative to the cell rect.
    private static let halfBlockRect: [Character: (CGFloat, CGFloat, CGFloat, CGFloat)] = [
        "▀": (0,   0,   1,   0.5),   // top half
        "▄": (0,   0.5, 1,   0.5),   // bottom half
        "▌": (0,   0,   0.5, 1  ),   // left half
        "▐": (0.5, 0,   0.5, 1  ),   // right half
    ]

    // MARK: - Draw

    /// Draw all cells into `ctx` at the given canvas size.
    ///
    /// Call from inside a `Canvas { ctx, size in … }` block.
    static func draw(cells: [VisualizerCell],
                     in ctx: inout GraphicsContext,
                     size: CGSize) {

        let cellW    = size.width  / CGFloat(cols)
        let cellH    = size.height / CGFloat(rows)
        // Text font — 95% of cell height to minimise gaps without clipping
        let fontSize = floor(min(cellW, cellH) * 0.95)
        let font     = Font.system(size: fontSize, design: .monospaced)

        for cell in cells {
            let x = CGFloat(cell.col) * cellW
            let y = CGFloat(cell.row) * cellH

            let color = Color(
                red:   Double(cell.r) / 255,
                green: Double(cell.g) / 255,
                blue:  Double(cell.b) / 255
            )

            let ch: Character = cell.ch.first ?? " "

            if let opacity = fullBlockOpacity[ch] {
                // ── Full-block: fill entire cell as a rectangle ───────────
                let rect = CGRect(x: x, y: y, width: cellW, height: cellH)
                ctx.fill(Path(rect), with: .color(color.opacity(opacity)))

            } else if let frac = halfBlockRect[ch] {
                // ── Half-block: fill a portion of the cell ────────────────
                let rect = CGRect(
                    x: x + frac.0 * cellW,
                    y: y + frac.1 * cellH,
                    width: frac.2 * cellW,
                    height: frac.3 * cellH
                )
                ctx.fill(Path(rect), with: .color(color))

            } else {
                // ── Text character: draw centred in the cell ──────────────
                ctx.draw(
                    Text(cell.ch)
                        .font(font)
                        .foregroundColor(color),
                    at: CGPoint(x: x + cellW * 0.5, y: y + cellH * 0.5),
                    anchor: .center
                )
            }
        }
    }

    // MARK: - Decode

    private static let decoder = JSONDecoder()

    /// Decode the JSON cell array produced by `aviz_render`.
    /// Returns an empty array on any parse error.
    static func decode(json: String) -> [VisualizerCell] {
        guard
            !json.isEmpty,
            json != "[]",
            let data = json.data(using: .utf8),
            let cells = try? decoder.decode([VisualizerCell].self, from: data)
        else { return [] }
        return cells
    }
}
