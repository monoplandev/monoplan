// A minimal MessagePack codec: exactly the subset `rmp_serde::to_vec_named`
// produces for the daemon's `Request` / `Reply` types (cli/src/ipc.rs),
// and nothing more. Hand-written rather than a dependency because the
// surface is small, the wire shape is pinned by a Rust test
// (`wire_shape_is_named_maps_and_bare_unit_variants`), and a tray should
// not pull a package graph for a dozen byte formats.
//
// Map keys are required to be strings: serde struct fields and variant
// names always are, and nothing else in the protocol is a map.

import Foundation

public indirect enum MPValue: Equatable {
    case `nil`
    case bool(Bool)
    case int(Int64)
    /// Only for values above `Int64.max`; everything else decodes to `.int`.
    case uint(UInt64)
    case double(Double)
    case string(String)
    case binary(Data)
    case array([MPValue])
    case map([String: MPValue])

    public var stringValue: String? {
        if case .string(let s) = self { return s }
        return nil
    }

    public var boolValue: Bool? {
        if case .bool(let b) = self { return b }
        return nil
    }

    public var intValue: Int64? {
        switch self {
        case .int(let i): return i
        case .uint(let u): return u <= UInt64(Int64.max) ? Int64(u) : nil
        default: return nil
        }
    }

    public var mapValue: [String: MPValue]? {
        if case .map(let m) = self { return m }
        return nil
    }

    public subscript(key: String) -> MPValue? {
        mapValue?[key]
    }
}

public enum MessagePackError: Error, Equatable {
    case truncated
    case unsupportedType(UInt8)
    case nonStringKey
    case invalidUTF8
    case trailingBytes(Int)
}

public enum MessagePack {
    // MARK: Encoding

    public static func encode(_ value: MPValue) -> Data {
        var out = Data()
        write(value, into: &out)
        return out
    }

    private static func write(_ value: MPValue, into out: inout Data) {
        switch value {
        case .nil:
            out.append(0xc0)
        case .bool(let b):
            out.append(b ? 0xc3 : 0xc2)
        case .int(let i):
            if i >= 0 {
                writeUInt(UInt64(i), into: &out)
            } else if i >= -32 {
                out.append(UInt8(truncatingIfNeeded: i))
            } else if i >= Int64(Int8.min) {
                out.append(0xd0)
                out.append(UInt8(bitPattern: Int8(i)))
            } else if i >= Int64(Int16.min) {
                out.append(0xd1)
                appendBE(UInt16(bitPattern: Int16(i)), to: &out)
            } else if i >= Int64(Int32.min) {
                out.append(0xd2)
                appendBE(UInt32(bitPattern: Int32(i)), to: &out)
            } else {
                out.append(0xd3)
                appendBE(UInt64(bitPattern: i), to: &out)
            }
        case .uint(let u):
            writeUInt(u, into: &out)
        case .double(let d):
            out.append(0xcb)
            appendBE(d.bitPattern, to: &out)
        case .string(let s):
            let bytes = Data(s.utf8)
            switch bytes.count {
            case 0...31:
                out.append(0xa0 | UInt8(bytes.count))
            case 32...255:
                out.append(0xd9)
                out.append(UInt8(bytes.count))
            case 256...65535:
                out.append(0xda)
                appendBE(UInt16(bytes.count), to: &out)
            default:
                out.append(0xdb)
                appendBE(UInt32(bytes.count), to: &out)
            }
            out.append(bytes)
        case .binary(let data):
            switch data.count {
            case 0...255:
                out.append(0xc4)
                out.append(UInt8(data.count))
            case 256...65535:
                out.append(0xc5)
                appendBE(UInt16(data.count), to: &out)
            default:
                out.append(0xc6)
                appendBE(UInt32(data.count), to: &out)
            }
            out.append(data)
        case .array(let items):
            switch items.count {
            case 0...15:
                out.append(0x90 | UInt8(items.count))
            case 16...65535:
                out.append(0xdc)
                appendBE(UInt16(items.count), to: &out)
            default:
                out.append(0xdd)
                appendBE(UInt32(items.count), to: &out)
            }
            for item in items { write(item, into: &out) }
        case .map(let entries):
            switch entries.count {
            case 0...15:
                out.append(0x80 | UInt8(entries.count))
            case 16...65535:
                out.append(0xde)
                appendBE(UInt16(entries.count), to: &out)
            default:
                out.append(0xdf)
                appendBE(UInt32(entries.count), to: &out)
            }
            // Sorted so encoding is deterministic (tests compare bytes);
            // serde does not care about field order.
            for key in entries.keys.sorted() {
                write(.string(key), into: &out)
                write(entries[key]!, into: &out)
            }
        }
    }

    private static func writeUInt(_ u: UInt64, into out: inout Data) {
        if u <= 0x7f {
            out.append(UInt8(u))
        } else if u <= UInt64(UInt8.max) {
            out.append(0xcc)
            out.append(UInt8(u))
        } else if u <= UInt64(UInt16.max) {
            out.append(0xcd)
            appendBE(UInt16(u), to: &out)
        } else if u <= UInt64(UInt32.max) {
            out.append(0xce)
            appendBE(UInt32(u), to: &out)
        } else {
            out.append(0xcf)
            appendBE(u, to: &out)
        }
    }

    private static func appendBE<T: FixedWidthInteger>(_ v: T, to out: inout Data) {
        var be = v.bigEndian
        withUnsafeBytes(of: &be) { out.append(contentsOf: $0) }
    }

    // MARK: Decoding

    public static func decode(_ data: Data) throws -> MPValue {
        var reader = Reader(data: data)
        let value = try reader.read()
        if reader.remaining > 0 {
            throw MessagePackError.trailingBytes(reader.remaining)
        }
        return value
    }

    private struct Reader {
        let data: Data
        var pos: Int

        init(data: Data) {
            self.data = data
            self.pos = data.startIndex
        }

        var remaining: Int { data.endIndex - pos }

        mutating func byte() throws -> UInt8 {
            guard pos < data.endIndex else { throw MessagePackError.truncated }
            let b = data[pos]
            pos += 1
            return b
        }

        mutating func bytes(_ n: Int) throws -> Data {
            guard remaining >= n else { throw MessagePackError.truncated }
            let slice = data[pos..<(pos + n)]
            pos += n
            return Data(slice)
        }

        mutating func be<T: FixedWidthInteger>(_: T.Type) throws -> T {
            let raw = try bytes(MemoryLayout<T>.size)
            return raw.withUnsafeBytes { $0.loadUnaligned(as: T.self) }.bigEndian
        }

        mutating func string(_ n: Int) throws -> String {
            guard let s = String(data: try bytes(n), encoding: .utf8) else {
                throw MessagePackError.invalidUTF8
            }
            return s
        }

        mutating func read() throws -> MPValue {
            let b = try byte()
            switch b {
            case 0x00...0x7f: return .int(Int64(b))
            case 0xe0...0xff: return .int(Int64(Int8(bitPattern: b)))
            case 0x80...0x8f: return try map(Int(b & 0x0f))
            case 0x90...0x9f: return try array(Int(b & 0x0f))
            case 0xa0...0xbf: return .string(try string(Int(b & 0x1f)))
            case 0xc0: return .nil
            case 0xc2: return .bool(false)
            case 0xc3: return .bool(true)
            case 0xc4: return .binary(try bytes(Int(try byte())))
            case 0xc5: return .binary(try bytes(Int(try be(UInt16.self))))
            case 0xc6: return .binary(try bytes(Int(try be(UInt32.self))))
            case 0xca: return .double(Double(Float(bitPattern: try be(UInt32.self))))
            case 0xcb: return .double(Double(bitPattern: try be(UInt64.self)))
            case 0xcc: return .int(Int64(try byte()))
            case 0xcd: return .int(Int64(try be(UInt16.self)))
            case 0xce: return .int(Int64(try be(UInt32.self)))
            case 0xcf:
                let u = try be(UInt64.self)
                return u <= UInt64(Int64.max) ? .int(Int64(u)) : .uint(u)
            case 0xd0: return .int(Int64(Int8(bitPattern: try byte())))
            case 0xd1: return .int(Int64(Int16(bitPattern: try be(UInt16.self))))
            case 0xd2: return .int(Int64(Int32(bitPattern: try be(UInt32.self))))
            case 0xd3: return .int(Int64(bitPattern: try be(UInt64.self)))
            case 0xd9: return .string(try string(Int(try byte())))
            case 0xda: return .string(try string(Int(try be(UInt16.self))))
            case 0xdb: return .string(try string(Int(try be(UInt32.self))))
            case 0xdc: return try array(Int(try be(UInt16.self)))
            case 0xdd: return try array(Int(try be(UInt32.self)))
            case 0xde: return try map(Int(try be(UInt16.self)))
            case 0xdf: return try map(Int(try be(UInt32.self)))
            default: throw MessagePackError.unsupportedType(b)
            }
        }

        mutating func array(_ n: Int) throws -> MPValue {
            var items: [MPValue] = []
            items.reserveCapacity(n)
            for _ in 0..<n { items.append(try read()) }
            return .array(items)
        }

        mutating func map(_ n: Int) throws -> MPValue {
            var entries: [String: MPValue] = [:]
            for _ in 0..<n {
                guard case .string(let key) = try read() else {
                    throw MessagePackError.nonStringKey
                }
                entries[key] = try read()
            }
            return .map(entries)
        }
    }
}
