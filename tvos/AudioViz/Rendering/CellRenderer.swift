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
//   font  ≈ 20 pt monospaced — fills ~83% of each cell
//
// Performance
// ───────────
// At 80×45 = 3 600 cells, drawing each character individually is the
// straightforward approach and comfortably within Apple TV's GPU budget at
// 45 fps.  If profiling ever shows a bottleneck, switch to a Metal-backed
// layer that uploads a texture atlas.

enum CellRenderer {

    // MARK: - Grid constants

    /// Character grid width used by the Rust core.
    static let cols = 80
    /// Character grid height used by the Rust core.
    static let rows = 45

    // MARK: - Draw

    /// Draw all cells into `ctx` at the given canvas size.
    ///
    /// Call from inside a `Canvas { ctx, size in … }` block.
    static func draw(cells: [VisualizerCell],
                     in ctx: inout GraphicsContext,
                     size: CGSize) {

        let cellW    = size.width  / CGFloat(cols)
        let cellH    = size.height / CGFloat(rows)
        // Keep the font slightly smaller than the cell so characters don't clip
        let fontSize = floor(min(cellW, cellH) * 0.82)
        let font     = Font.system(size: fontSize, design: .monospaced)

        for cell in cells {
            // Centre the character within its cell
            let x = CGFloat(cell.col) * cellW + cellW * 0.5
            let y = CGFloat(cell.row) * cellH + cellH * 0.5

            let color = Color(
                red:   Double(cell.r) / 255,
                green: Double(cell.g) / 255,
                blue:  Double(cell.b) / 255
            )

            ctx.draw(
                Text(cell.ch)
                    .font(font)
                    .foregroundColor(color),
                at: CGPoint(x: x, y: y),
                anchor: .center
            )
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
