// SPDX-License-Identifier: BSL-1.1

/**
 * Minimal MessagePack codec.
 *
 * The RaisinDB WebSocket transport speaks **MessagePack over binary frames**,
 * not JSON — the server decodes every inbound frame with
 * `rmp_serde::from_slice` and explicitly drops text frames with a warning
 * (`raisin-transport-ws/src/handler/socket.rs`). So a browser client that
 * sends `JSON.stringify(...)` is silently ignored: no response, no error.
 *
 * Why hand-rolled rather than `@msgpack/msgpack`: this package cannot be
 * `npm install`ed in place. `package.json` pins `@raisindb/sql-wasm` with the
 * `link:` protocol, which the installed npm rejects with `EUNSUPPORTEDPROTOCOL`
 * before it resolves anything else, so *adding any dependency at all* first
 * requires unrelated build-tooling surgery. The subset of MessagePack the
 * transport actually uses is small and fully specified, so it is encoded here
 * directly and the console keeps a zero-install dependency graph.
 *
 * Scope: the whole format is decoded (`rmp_serde` picks the narrowest
 * encoding for every value, so a decoder cannot be partial), while the encoder
 * emits only what a request envelope contains — nil, bool, int, float, string,
 * binary, array, map.
 *
 * Not supported, deliberately: the timestamp extension type. `rmp_serde`
 * serializes `chrono` values as ISO-8601 *strings* rather than ext 0xff, so it
 * never appears on this wire. Ext payloads decode to a tagged
 * {@link MsgpackExt} object rather than throwing, so an unexpected one is
 * inspectable instead of fatal.
 */

/** An unrecognized MessagePack extension value, preserved rather than dropped. */
export interface MsgpackExt {
  __msgpackExt: number
  data: Uint8Array
}

// ============================================================
// Encoder
// ============================================================

class Writer {
  private buf = new Uint8Array(1024)
  private len = 0

  private reserve(n: number) {
    if (this.len + n <= this.buf.length) return
    let next = this.buf.length * 2
    while (next < this.len + n) next *= 2
    const grown = new Uint8Array(next)
    grown.set(this.buf.subarray(0, this.len))
    this.buf = grown
  }

  u8(v: number) {
    this.reserve(1)
    this.buf[this.len++] = v
  }

  bytes(b: Uint8Array) {
    this.reserve(b.length)
    this.buf.set(b, this.len)
    this.len += b.length
  }

  /** Big-endian write of `width` bytes. */
  be(value: number, width: number) {
    this.reserve(width)
    for (let i = width - 1; i >= 0; i--) {
      this.buf[this.len + i] = value & 0xff
      value = Math.floor(value / 256)
    }
    this.len += width
  }

  f64(value: number) {
    this.reserve(8)
    new DataView(this.buf.buffer, this.buf.byteOffset + this.len, 8).setFloat64(0, value, false)
    this.len += 8
  }

  finish(): Uint8Array {
    return this.buf.slice(0, this.len)
  }
}

const utf8Encode = new TextEncoder()

/** Emit a format byte followed by a big-endian payload of `width` bytes. */
function tagged(w: Writer, tag: number, value: number, width: number) {
  w.u8(tag)
  w.be(value, width)
}

function encodeInt(w: Writer, n: number) {
  if (n >= 0) {
    if (n < 0x80) return w.u8(n) // positive fixint
    if (n <= 0xff) return tagged(w, 0xcc, n, 1)
    if (n <= 0xffff) return tagged(w, 0xcd, n, 2)
    if (n <= 0xffffffff) return tagged(w, 0xce, n, 4)
    return tagged(w, 0xcf, n, 8) // uint64
  }
  if (n >= -32) return w.u8(0xe0 | (n + 32)) // negative fixint
  if (n >= -0x80) return tagged(w, 0xd0, n & 0xff, 1)
  if (n >= -0x8000) return tagged(w, 0xd1, n & 0xffff, 2)
  if (n >= -0x80000000) return tagged(w, 0xd2, n >>> 0, 4)
  // int64: two's complement over 8 bytes via BigInt to keep the high word exact.
  w.u8(0xd3)
  const big = BigInt.asUintN(64, BigInt(n))
  w.bytes(bigToBytes(big, 8))
}

function bigToBytes(v: bigint, width: number): Uint8Array {
  const out = new Uint8Array(width)
  for (let i = width - 1; i >= 0; i--) {
    out[i] = Number(v & 0xffn)
    v >>= 8n
  }
  return out
}

function encodeValue(w: Writer, v: unknown) {
  if (v === null || v === undefined) return w.u8(0xc0)
  if (typeof v === 'boolean') return w.u8(v ? 0xc3 : 0xc2)

  if (typeof v === 'number') {
    if (Number.isSafeInteger(v)) return encodeInt(w, v)
    w.u8(0xcb)
    return w.f64(v)
  }

  if (typeof v === 'bigint') {
    w.u8(v >= 0n ? 0xcf : 0xd3)
    return w.bytes(bigToBytes(BigInt.asUintN(64, v), 8))
  }

  if (typeof v === 'string') {
    const b = utf8Encode.encode(v)
    if (b.length < 32) w.u8(0xa0 | b.length)
    else if (b.length <= 0xff) tagged(w, 0xd9, b.length, 1)
    else if (b.length <= 0xffff) tagged(w, 0xda, b.length, 2)
    else tagged(w, 0xdb, b.length, 4)
    return w.bytes(b)
  }

  if (v instanceof Uint8Array) {
    if (v.length <= 0xff) tagged(w, 0xc4, v.length, 1)
    else if (v.length <= 0xffff) tagged(w, 0xc5, v.length, 2)
    else tagged(w, 0xc6, v.length, 4)
    return w.bytes(v)
  }

  if (Array.isArray(v)) {
    if (v.length < 16) w.u8(0x90 | v.length)
    else if (v.length <= 0xffff) tagged(w, 0xdc, v.length, 2)
    else tagged(w, 0xdd, v.length, 4)
    for (const item of v) encodeValue(w, item)
    return
  }

  // Plain object → map. `undefined`-valued keys are dropped rather than
  // encoded as nil, mirroring `serde`'s `skip_serializing_if = "Option::is_none"`
  // on the envelope's optional context fields.
  const entries = Object.entries(v as Record<string, unknown>).filter(([, val]) => val !== undefined)
  if (entries.length < 16) w.u8(0x80 | entries.length)
  else if (entries.length <= 0xffff) tagged(w, 0xde, entries.length, 2)
  else tagged(w, 0xdf, entries.length, 4)
  for (const [key, val] of entries) {
    encodeValue(w, key)
    encodeValue(w, val)
  }
}

/** Encode a value to a MessagePack frame. */
export function msgpackEncode(value: unknown): Uint8Array {
  const w = new Writer()
  encodeValue(w, value)
  return w.finish()
}

// ============================================================
// Decoder
// ============================================================

const utf8Decode = new TextDecoder('utf-8')

class Reader {
  private view: DataView
  private bytes: Uint8Array
  private pos = 0

  constructor(data: ArrayBuffer | Uint8Array) {
    this.bytes = data instanceof Uint8Array ? data : new Uint8Array(data)
    this.view = new DataView(this.bytes.buffer, this.bytes.byteOffset, this.bytes.byteLength)
  }

  private need(n: number) {
    if (this.pos + n > this.bytes.length) {
      throw new Error(`MessagePack: truncated frame (need ${n} bytes at offset ${this.pos})`)
    }
  }

  u8(): number {
    this.need(1)
    return this.view.getUint8(this.pos++)
  }

  private take(n: number): Uint8Array {
    this.need(n)
    const out = this.bytes.subarray(this.pos, this.pos + n)
    this.pos += n
    return out
  }

  private uint(width: number): number {
    this.need(width)
    let v = 0
    for (let i = 0; i < width; i++) v = v * 256 + this.view.getUint8(this.pos + i)
    this.pos += width
    return v
  }

  /**
   * 64-bit reads go through BigInt so the high word is never mangled, then
   * narrow to a JS number when exact. HLC revisions and epoch seconds are
   * always in range; anything larger is handed back as a BigInt rather than
   * silently rounded.
   */
  private u64(): number | bigint {
    this.need(8)
    const v = this.view.getBigUint64(this.pos, false)
    this.pos += 8
    return v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v
  }

  private i64(): number | bigint {
    this.need(8)
    const v = this.view.getBigInt64(this.pos, false)
    this.pos += 8
    const min = BigInt(Number.MIN_SAFE_INTEGER)
    const max = BigInt(Number.MAX_SAFE_INTEGER)
    return v >= min && v <= max ? Number(v) : v
  }

  private int(width: number): number {
    this.need(width)
    let v: number
    if (width === 1) v = this.view.getInt8(this.pos)
    else if (width === 2) v = this.view.getInt16(this.pos, false)
    else v = this.view.getInt32(this.pos, false)
    this.pos += width
    return v
  }

  private str(len: number): string {
    return utf8Decode.decode(this.take(len))
  }

  private array(len: number): unknown[] {
    const out = new Array(len)
    for (let i = 0; i < len; i++) out[i] = this.value()
    return out
  }

  private map(len: number): Record<string, unknown> {
    const out: Record<string, unknown> = {}
    for (let i = 0; i < len; i++) {
      const key = this.value()
      // Non-string keys cannot occur here (`to_vec_named` always emits struct
      // field names) but must not corrupt the object if they ever do.
      out[typeof key === 'string' ? key : String(key)] = this.value()
    }
    return out
  }

  private ext(len: number): MsgpackExt {
    const type = this.u8()
    return { __msgpackExt: type, data: new Uint8Array(this.take(len)) }
  }

  value(): unknown {
    const b = this.u8()

    // Single-byte families
    if (b <= 0x7f) return b // positive fixint
    if (b >= 0xe0) return b - 256 // negative fixint
    if (b >= 0x80 && b <= 0x8f) return this.map(b & 0x0f) // fixmap
    if (b >= 0x90 && b <= 0x9f) return this.array(b & 0x0f) // fixarray
    if (b >= 0xa0 && b <= 0xbf) return this.str(b & 0x1f) // fixstr

    switch (b) {
      case 0xc0: return null
      case 0xc2: return false
      case 0xc3: return true

      case 0xc4: return new Uint8Array(this.take(this.uint(1)))
      case 0xc5: return new Uint8Array(this.take(this.uint(2)))
      case 0xc6: return new Uint8Array(this.take(this.uint(4)))

      case 0xc7: { const l = this.uint(1); return this.ext(l) }
      case 0xc8: { const l = this.uint(2); return this.ext(l) }
      case 0xc9: { const l = this.uint(4); return this.ext(l) }

      case 0xca: { this.need(4); const v = this.view.getFloat32(this.pos, false); this.pos += 4; return v }
      case 0xcb: { this.need(8); const v = this.view.getFloat64(this.pos, false); this.pos += 8; return v }

      case 0xcc: return this.uint(1)
      case 0xcd: return this.uint(2)
      case 0xce: return this.uint(4)
      case 0xcf: return this.u64()

      case 0xd0: return this.int(1)
      case 0xd1: return this.int(2)
      case 0xd2: return this.int(4)
      case 0xd3: return this.i64()

      case 0xd4: return this.ext(1)
      case 0xd5: return this.ext(2)
      case 0xd6: return this.ext(4)
      case 0xd7: return this.ext(8)
      case 0xd8: return this.ext(16)

      case 0xd9: return this.str(this.uint(1))
      case 0xda: return this.str(this.uint(2))
      case 0xdb: return this.str(this.uint(4))

      case 0xdc: return this.array(this.uint(2))
      case 0xdd: return this.array(this.uint(4))

      case 0xde: return this.map(this.uint(2))
      case 0xdf: return this.map(this.uint(4))

      default:
        throw new Error(`MessagePack: unknown format byte 0x${b.toString(16)}`)
    }
  }
}

/** Decode a MessagePack frame. */
export function msgpackDecode(data: ArrayBuffer | Uint8Array): unknown {
  return new Reader(data).value()
}
