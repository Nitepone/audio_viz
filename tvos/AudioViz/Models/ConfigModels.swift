import Foundation

// ── Config models ─────────────────────────────────────────────────────────────
//
// Mirror the JSON schema produced by Rust's `get_default_config()`:
//
//   {
//     "visualizer_name": "spectrum",
//     "version": 1,
//     "config": [
//       {"name":"gain",  "display_name":"Gain",  "type":"float",
//        "value":1.0, "min":0.0, "max":4.0},
//       {"name":"theme", "display_name":"Theme", "type":"enum",
//        "value":"hifi", "variants":["classic","hifi","led"]},
//       {"name":"mirror","display_name":"Mirror","type":"bool",
//        "value":true}
//     ]
//   }
//
// The same JSON (with mutated "value" fields) is sent back to `set_config()`.

// MARK: - Root

struct ConfigRoot: Codable {
    let visualizer_name: String?
    let version:         Int?
    var config:          [ConfigItem]
}

// MARK: - Item

struct ConfigItem: Codable, Identifiable {
    var id: String { name }

    let name:         String
    let display_name: String
    let type:         String      // "float" | "int" | "enum" | "bool"
    var value:        JSONValue
    // float fields
    let min:      Double?
    let max:      Double?
    // enum fields
    let variants: [String]?
}

// MARK: - Heterogeneous value (float, string, or bool)

enum JSONValue: Codable {
    case double(Double)
    case string(String)
    case bool(Bool)

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        // Attempt Bool before Double — JSON true/false are distinct tokens,
        // but some decoders coerce; ordering here keeps semantics correct.
        if let b = try? c.decode(Bool.self)   { self = .bool(b);   return }
        if let d = try? c.decode(Double.self) { self = .double(d); return }
        if let s = try? c.decode(String.self) { self = .string(s); return }
        throw DecodingError.dataCorruptedError(
            in: c, debugDescription: "ConfigItem value must be Bool, Double, or String")
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .double(let d): try c.encode(d)
        case .string(let s): try c.encode(s)
        case .bool(let b):   try c.encode(b)
        }
    }

    // MARK: Accessors

    var doubleValue: Double? {
        guard case .double(let d) = self else { return nil }
        return d
    }
    var stringValue: String? {
        guard case .string(let s) = self else { return nil }
        return s
    }
    var boolValue: Bool? {
        guard case .bool(let b) = self else { return nil }
        return b
    }
}
