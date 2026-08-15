//! The ArchiveLib "GL" compressed bitstream — decoder plus two
//! encoders (literal-only and match-bearing).
//!
//! Husqvarna Viking HUS and VIP compress their three stitch streams
//! with the "GL" method of a mid-1990s general-purpose compression
//! library, documented bitstream-level by the workspace's staged
//! clean-room material (`docs/embroidery/hus/archivelib-gl-bitstream.md`,
//! validated bit-exactly over 33 real streams). Because the library is
//! general-purpose, nothing in this module is embroidery-specific.
//!
//! GL is a block-structured static-Huffman LZSS coder of the LZH
//! family: a sliding history window, matches coded as
//! (length, distance) pairs, and per-block canonical-Huffman code sets
//! transmitted as Huffman-coded code-length lists. Bits are read
//! MSB-first. A stream carries no header, no magic and no length
//! field; it ends on the end-of-stream code (510).
//!
//! [`compress`] emits **literal-only** streams (every block codes
//! literals plus the final end-of-stream code, never a match) — the
//! same shape every real HUS/VIP producer in the validation corpus
//! emits, and the shape the HUS/VIP encoders keep. [`compress_lz`]
//! additionally emits the documented match layer (length codes
//! 256…509, the offset code, self-extending copies) over a chosen
//! history window; the staged documentation derives those rules from
//! the vendor binary and notes no real HUS/VIP stream exercises
//! them, so the match-bearing encoder exists to exercise the decoder
//! against exactly the documented rules (and for general-purpose
//! use), not to imitate a corpus producer.

use crate::{Error, Result};

/// The library's default history window (compressor level 4).
pub const DEFAULT_WINDOW: usize = 16384;
/// Literal/length alphabet size (codes 0…510).
const LIT_ALPHABET: usize = 511;
/// End-of-stream code.
const EOS: usize = 510;
/// Code-length alphabet size.
const LEN_ALPHABET: usize = 19;
/// Offset alphabet size (fixed regardless of window size).
const OFF_ALPHABET: usize = 15;
/// Longest canonical code this implementation accepts or emits.
const MAX_CODE_LEN: u8 = 16;

// ───────────────────────── bit I/O ─────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    /// Next byte to load into the accumulator.
    pos: usize,
    /// MSB-aligned pending bits (top `nbits` of a conceptual stream).
    acc: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            acc: 0,
            nbits: 0,
        }
    }

    /// Reads `n` bits MSB-first (`n` ≤ 32).
    fn read(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 32);
        while self.nbits < n {
            let b = *self.data.get(self.pos).ok_or(Error::UnexpectedEof {
                context: "GL bitstream",
            })?;
            self.pos += 1;
            self.acc = (self.acc << 8) | b as u64;
            self.nbits += 8;
        }
        let v = (self.acc >> (self.nbits - n)) as u32 & ((1u64 << n) - 1) as u32;
        self.nbits -= n;
        Ok(v)
    }

    /// Bytes of input consumed so far (partially-read bytes count).
    fn bytes_consumed(&self) -> usize {
        self.pos - (self.nbits / 8) as usize
    }
}

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    fn write(&mut self, v: u32, n: u32) {
        debug_assert!(n <= 32 && (n == 32 || v < (1 << n)));
        self.acc = (self.acc << n) | v as u64;
        self.nbits += n;
        while self.nbits >= 8 {
            self.out.push((self.acc >> (self.nbits - 8)) as u8);
            self.nbits -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            let pad = 8 - self.nbits;
            self.acc <<= pad;
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

// ───────────────────────── canonical codes ─────────────────────────

/// One decodable code set: either a degenerate set (every lookup
/// yields the same symbol while consuming no bits) or a canonical
/// Huffman code described by its code lengths.
enum CodeSet {
    Degenerate(u32),
    Canonical(Box<Canonical>),
}

struct Canonical {
    /// `first_code[l]` — the first canonical code of length `l`.
    first_code: [u32; MAX_CODE_LEN as usize + 1],
    /// `first_index[l]` — index into `symbols` of that code.
    first_index: [u32; MAX_CODE_LEN as usize + 1],
    /// `count[l]` — number of codes of length `l`.
    count: [u32; MAX_CODE_LEN as usize + 1],
    /// Symbols ordered by (length, symbol) — canonical order.
    symbols: Vec<u16>,
}

impl CodeSet {
    /// Builds the canonical code from per-symbol lengths (0 = unused).
    /// Canonical assignment: shortest length first, increasing symbol
    /// order within each length, MSB-first.
    fn from_lengths(lengths: &[u8]) -> Result<CodeSet> {
        let mut count = [0u32; MAX_CODE_LEN as usize + 1];
        for &l in lengths {
            if l > MAX_CODE_LEN {
                return Err(Error::invalid(format!("GL code length {l} too long")));
            }
            if l > 0 {
                count[l as usize] += 1;
            }
        }
        let mut first_code = [0u32; MAX_CODE_LEN as usize + 1];
        let mut first_index = [0u32; MAX_CODE_LEN as usize + 1];
        let mut code = 0u32;
        let mut index = 0u32;
        for l in 1..=MAX_CODE_LEN as usize {
            first_code[l] = code;
            first_index[l] = index;
            // Over-subscribed length sets cannot form a prefix code.
            if code + count[l] > (1u32 << l) {
                return Err(Error::invalid("GL code lengths over-subscribe the code"));
            }
            code = (code + count[l]) << 1;
            index += count[l];
        }
        let mut symbols = vec![0u16; index as usize];
        let mut next = first_index;
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 {
                symbols[next[l as usize] as usize] = sym as u16;
                next[l as usize] += 1;
            }
        }
        Ok(CodeSet::Canonical(Box::new(Canonical {
            first_code,
            first_index,
            count,
            symbols,
        })))
    }

    /// Decodes one symbol.
    fn decode(&self, r: &mut BitReader) -> Result<u32> {
        match self {
            CodeSet::Degenerate(v) => Ok(*v),
            CodeSet::Canonical(c) => {
                let mut code = 0u32;
                for l in 1..=MAX_CODE_LEN as usize {
                    code = (code << 1) | r.read(1)?;
                    let rel = code.wrapping_sub(c.first_code[l]);
                    if c.count[l] > 0 && rel < c.count[l] {
                        return Ok(c.symbols[(c.first_index[l] + rel) as usize] as u32);
                    }
                }
                Err(Error::invalid("GL bit pattern matches no code"))
            }
        }
    }
}

// ───────────────────────── code-length readers ─────────────────────────

/// Reads the 19- or 15-symbol code-length set: 5-bit count, 3-bit
/// lengths with a unary extension at 7, and (19-symbol set only) a
/// 2-bit forced-zero run once the running index reaches 3.
fn read_small_set(r: &mut BitReader, alphabet: usize, special_at_3: bool) -> Result<CodeSet> {
    let n = r.read(5)? as usize;
    if n == 0 {
        return Ok(CodeSet::Degenerate(r.read(5)?));
    }
    if n > alphabet {
        return Err(Error::invalid(format!(
            "GL code-length count {n} exceeds the {alphabet}-symbol alphabet"
        )));
    }
    let mut lengths = vec![0u8; alphabet];
    let mut i = 0usize;
    let mut special_done = false;
    while i < n {
        if special_at_3 && i == 3 && !special_done {
            special_done = true;
            let zeros = r.read(2)? as usize;
            i += zeros; // the skipped lengths stay zero
            continue;
        }
        let mut v = r.read(3)?;
        if v == 7 {
            while r.read(1)? == 1 {
                v += 1;
                if v > MAX_CODE_LEN as u32 {
                    return Err(Error::invalid("GL unary code length overruns"));
                }
            }
        }
        if i < alphabet {
            lengths[i] = v as u8;
        }
        i += 1;
    }
    CodeSet::from_lengths(&lengths)
}

/// Reads the 511-symbol literal/length code-length set: 9-bit count,
/// then run-length coded lengths via the 19-symbol set.
fn read_literal_set(r: &mut BitReader, small: &CodeSet) -> Result<CodeSet> {
    let n = r.read(9)? as usize;
    if n == 0 {
        return Ok(CodeSet::Degenerate(r.read(9)?));
    }
    if n > LIT_ALPHABET {
        return Err(Error::invalid(format!(
            "GL literal code-length count {n} exceeds the alphabet"
        )));
    }
    let mut lengths = vec![0u8; LIT_ALPHABET];
    let mut i = 0usize;
    while i < n {
        let v = small.decode(r)?;
        match v {
            0 => i += 1,
            1 => i += r.read(4)? as usize + 3,
            2 => i += r.read(9)? as usize + 20,
            _ => {
                if v - 2 > MAX_CODE_LEN as u32 {
                    return Err(Error::invalid("GL literal code length too long"));
                }
                if i < LIT_ALPHABET {
                    lengths[i] = (v - 2) as u8;
                }
                i += 1;
            }
        }
    }
    CodeSet::from_lengths(&lengths)
}

// ───────────────────────── decompression ─────────────────────────

/// Decompresses a GL stream with the default 16 KiB window, stopping
/// at the end-of-stream code. `max_out` bounds the output length
/// (the stream itself carries no length, so hostile inputs would
/// otherwise expand without limit).
///
/// Returns the output and the number of input bytes consumed
/// (partially-used trailing bytes count as consumed).
pub fn decompress(data: &[u8], max_out: usize) -> Result<(Vec<u8>, usize)> {
    decompress_with_window(data, DEFAULT_WINDOW, max_out)
}

/// [`decompress`] with an explicit history-window size. The window is
/// a compressor property not recorded in the stream; the library
/// accepts 1024…16384 (powers of two, level 0…4).
pub fn decompress_with_window(
    data: &[u8],
    window: usize,
    max_out: usize,
) -> Result<(Vec<u8>, usize)> {
    if !(1024..=16384).contains(&window) || !window.is_power_of_two() {
        return Err(Error::invalid(format!("GL window size {window}")));
    }
    let mut r = BitReader::new(data);
    let mut out = Vec::new();
    let mut ring = vec![0u8; window];
    let mut wpos = 0usize;
    let push = |out: &mut Vec<u8>, ring: &mut Vec<u8>, wpos: &mut usize, b: u8| {
        out.push(b);
        ring[*wpos] = b;
        *wpos = (*wpos + 1) & (window - 1);
    };
    loop {
        // Block header: 16-bit code count, then the three code sets.
        let block_codes = r.read(16)? as usize;
        let small = read_small_set(&mut r, LEN_ALPHABET, true)?;
        let lit = read_literal_set(&mut r, &small)?;
        let off = read_small_set(&mut r, OFF_ALPHABET, false)?;
        for _ in 0..block_codes {
            let sym = lit.decode(&mut r)? as usize;
            match sym {
                0..=255 => {
                    if out.len() >= max_out {
                        return Err(Error::invalid("GL output exceeds the caller's limit"));
                    }
                    push(&mut out, &mut ring, &mut wpos, sym as u8);
                }
                EOS => return Ok((out, r.bytes_consumed())),
                256..=509 => {
                    let len = sym - 253; // 3…256
                    let j = off.decode(&mut r)? as usize;
                    if j >= OFF_ALPHABET {
                        return Err(Error::invalid("GL offset symbol out of range"));
                    }
                    let distance = if j == 0 {
                        0
                    } else {
                        (1usize << (j - 1)) + r.read((j - 1) as u32)? as usize
                    };
                    if out.len() + len > max_out {
                        return Err(Error::invalid("GL output exceeds the caller's limit"));
                    }
                    // Source is (write_pos − distance − 1) mod window;
                    // byte-at-a-time copy, so a match may overlap its
                    // own output and self-extend.
                    let mut src = (wpos + window - 1 - (distance & (window - 1))) & (window - 1);
                    for _ in 0..len {
                        let b = ring[src];
                        src = (src + 1) & (window - 1);
                        push(&mut out, &mut ring, &mut wpos, b);
                    }
                }
                _ => return Err(Error::invalid("GL symbol outside the 511-code alphabet")),
            }
        }
        // Code count exhausted without the end-of-stream code: the
        // next block header follows at the current bit position.
    }
}

// ───────────────────────── compression ─────────────────────────

/// Symbols per block. The block header's code count is 16-bit, so a
/// block holds at most 65 535 codes; stay comfortably below.
const BLOCK_CODES: usize = 32768;
/// Shortest codable match (length code 256).
const MIN_MATCH: usize = 3;
/// Longest codable match (length code 509).
const MAX_MATCH: usize = 256;

/// One LZSS token of the pre-entropy stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    /// A literal byte.
    Lit(u8),
    /// A copy of `len` bytes (3…256) whose source starts `dist + 1`
    /// bytes before the write position (the documented distance rule),
    /// so `dist == 0` re-reads the byte just written and self-extends.
    Match { len: u16, dist: u16 },
}

/// Splits a match distance into (offset symbol, extra value, extra
/// bit count) per the documented rule: symbol 0 is distance 0;
/// symbol *j* > 0 covers distances 2^(*j*−1) … 2^*j* − 1 with
/// *j* − 1 literal extra bits.
fn offset_parts(dist: u16) -> (usize, u32, u32) {
    if dist == 0 {
        (0, 0, 0)
    } else {
        let j = (16 - dist.leading_zeros()) as usize;
        (j, u32::from(dist) - (1 << (j - 1)), (j - 1) as u32)
    }
}

/// Builds length-limited Huffman code lengths for `freq` (0 = unused).
/// Falls back to equal-length codes if the unrestricted Huffman tree
/// exceeds [`MAX_CODE_LEN`] (practically unreachable for these
/// alphabet sizes, but the fallback keeps the encoder total).
fn huffman_lengths(freq: &[u32]) -> Vec<u8> {
    let used: Vec<usize> = (0..freq.len()).filter(|&s| freq[s] > 0).collect();
    let mut lengths = vec![0u8; freq.len()];
    match used.len() {
        0 => return lengths,
        1 => {
            lengths[used[0]] = 1;
            return lengths;
        }
        _ => {}
    }
    // Plain two-queue Huffman over (weight, node).
    #[derive(Clone)]
    enum Node {
        Leaf(usize),
        Join(Box<Node>, Box<Node>),
    }
    let mut heap: std::collections::BinaryHeap<(std::cmp::Reverse<u64>, usize)> =
        std::collections::BinaryHeap::new();
    let mut nodes: Vec<Node> = Vec::new();
    for &s in &used {
        nodes.push(Node::Leaf(s));
        heap.push((std::cmp::Reverse(freq[s] as u64), nodes.len() - 1));
    }
    while heap.len() > 1 {
        let (std::cmp::Reverse(w1), i1) = heap.pop().unwrap();
        let (std::cmp::Reverse(w2), i2) = heap.pop().unwrap();
        let joined = Node::Join(Box::new(nodes[i1].clone()), Box::new(nodes[i2].clone()));
        nodes.push(joined);
        heap.push((std::cmp::Reverse(w1 + w2), nodes.len() - 1));
    }
    fn walk(n: &Node, depth: u8, lengths: &mut [u8], too_deep: &mut bool) {
        match n {
            Node::Leaf(s) => {
                if depth > MAX_CODE_LEN {
                    *too_deep = true;
                } else {
                    lengths[*s] = depth.max(1);
                }
            }
            Node::Join(a, b) => {
                walk(a, depth + 1, lengths, too_deep);
                walk(b, depth + 1, lengths, too_deep);
            }
        }
    }
    let mut too_deep = false;
    let root = heap.pop().unwrap().1;
    walk(&nodes[root], 0, &mut lengths, &mut too_deep);
    if too_deep {
        // Equal-length fallback: ceil(log2(k)) bits for every symbol.
        let bits = (usize::BITS - (used.len() - 1).leading_zeros()).max(1) as u8;
        lengths.fill(0);
        for &s in &used {
            lengths[s] = bits;
        }
    }
    lengths
}

/// Writes a 3-bit length with the unary extension at 7.
fn write_small_len(w: &mut BitWriter, l: u8) {
    if l < 7 {
        w.write(l as u32, 3);
    } else {
        w.write(7, 3);
        for _ in 7..l {
            w.write(1, 1);
        }
        w.write(0, 1);
    }
}

/// The run-length coded transmission of the literal set's lengths, as
/// (symbol, extra-bits, extra-count) triples over the 19-set.
fn literal_transmission(lengths: &[u8]) -> (usize, Vec<(u8, u32, u32)>) {
    let n = lengths
        .iter()
        .rposition(|&l| l > 0)
        .map(|p| p + 1)
        .unwrap_or(0);
    let mut items = Vec::new();
    let mut i = 0usize;
    while i < n {
        if lengths[i] == 0 {
            let mut run = 1usize;
            while i + run < n && lengths[i + run] == 0 {
                run += 1;
            }
            let mut left = run;
            while left >= 20 {
                let take = left.min(20 + 511);
                items.push((2, (take - 20) as u32, 9));
                left -= take;
            }
            while left >= 3 {
                let take = left.min(3 + 15);
                items.push((1, (take - 3) as u32, 4));
                left -= take;
            }
            for _ in 0..left {
                items.push((0, 0, 0));
            }
            i += run;
        } else {
            items.push((lengths[i] + 2, 0, 0));
            i += 1;
        }
    }
    (n, items)
}

/// Entropy-codes a token stream as GL blocks — the shared back half
/// of both encoders. The final block carries the end-of-stream code.
fn write_tokens(tokens: &[Token]) -> Vec<u8> {
    let mut w = BitWriter::new();
    let mut chunks: Vec<&[Token]> = tokens.chunks(BLOCK_CODES).collect();
    if chunks.is_empty() {
        chunks.push(&[]);
    }
    let last = chunks.len() - 1;
    for (ci, chunk) in chunks.iter().enumerate() {
        let is_last = ci == last;
        // Literal/length code: literal bytes and match-length codes
        // (256…509 = length + 253), plus EOS on the final block; the
        // offset code over the matches' offset symbols.
        let mut lit_freq = vec![0u32; LIT_ALPHABET];
        let mut off_freq = vec![0u32; OFF_ALPHABET];
        for t in chunk.iter() {
            match *t {
                Token::Lit(b) => lit_freq[b as usize] += 1,
                Token::Match { len, dist } => {
                    lit_freq[len as usize + 253] += 1;
                    off_freq[offset_parts(dist).0] += 1;
                }
            }
        }
        if is_last {
            lit_freq[EOS] += 1;
        }
        let lit_lengths = huffman_lengths(&lit_freq);
        let (lit_n, items) = literal_transmission(&lit_lengths);

        // 19-symbol code over the transmission items.
        let mut small_freq = vec![0u32; LEN_ALPHABET];
        for &(s, _, _) in &items {
            small_freq[s as usize] += 1;
        }
        let small_lengths = huffman_lengths(&small_freq);
        let small_codes = codes_from_lengths(&small_lengths);
        let lit_codes = codes_from_lengths(&lit_lengths);
        let off_lengths = huffman_lengths(&off_freq);
        let off_codes = codes_from_lengths(&off_lengths);

        // Block header: code count.
        let count = chunk.len() + usize::from(is_last);
        w.write(count as u32, 16);

        // 19-symbol lengths: 5-bit count, 3-bit lengths (unary
        // extension), the 2-bit forced-zero field once the index
        // reaches 3.
        let n19 = small_lengths
            .iter()
            .rposition(|&l| l > 0)
            .map(|p| p + 1)
            .unwrap_or(1);
        w.write(n19 as u32, 5);
        let mut i = 0usize;
        let mut special_done = false;
        while i < n19 {
            if i == 3 && !special_done {
                special_done = true;
                w.write(0, 2); // no forced zeros
                continue;
            }
            write_small_len(&mut w, small_lengths[i]);
            i += 1;
        }

        // Literal lengths: 9-bit count, run-length coded via the
        // 19-symbol code.
        w.write(lit_n as u32, 9);
        for &(s, extra, extra_bits) in &items {
            let (code, len) = small_codes[s as usize];
            w.write(code, len as u32);
            if extra_bits > 0 {
                w.write(extra, extra_bits);
            }
        }

        // Offset set: degenerate when the block codes no match (the
        // decoder then never consults it); otherwise a plain
        // 15-symbol length list — its reader has no forced-zero
        // position.
        match off_lengths.iter().rposition(|&l| l > 0) {
            None => {
                w.write(0, 5);
                w.write(0, 5);
            }
            Some(p) => {
                let n15 = p + 1;
                w.write(n15 as u32, 5);
                for &l in &off_lengths[..n15] {
                    write_small_len(&mut w, l);
                }
            }
        }

        // The codes themselves.
        for t in chunk.iter() {
            match *t {
                Token::Lit(b) => {
                    let (code, len) = lit_codes[b as usize];
                    w.write(code, len as u32);
                }
                Token::Match { len: mlen, dist } => {
                    let (code, len) = lit_codes[mlen as usize + 253];
                    w.write(code, len as u32);
                    let (j, extra, extra_bits) = offset_parts(dist);
                    let (code, len) = off_codes[j];
                    w.write(code, len as u32);
                    if extra_bits > 0 {
                        w.write(extra, extra_bits);
                    }
                }
            }
        }
        if is_last {
            let (code, len) = lit_codes[EOS];
            w.write(code, len as u32);
        }
    }
    w.finish()
}

/// Compresses `data` as a GL stream (literal-only blocks, default
/// window semantics — literal streams are window-independent). This
/// is the shape every real HUS/VIP producer emits, and the encoder
/// the HUS/VIP writers use.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let tokens: Vec<Token> = data.iter().map(|&b| Token::Lit(b)).collect();
    write_tokens(&tokens)
}

/// Compresses `data` as a **match-bearing** GL stream over the given
/// history window (1024…16384, a power of two — compressor levels
/// 0…4; the library default is 16384).
///
/// The staged documentation derives the match and offset rules from
/// the vendor binary and notes that no real HUS/VIP stream exercises
/// them — [`compress`] therefore stays the producer-shaped encoder,
/// while this one codes the documented match layer in full: length
/// codes 256…509, offset symbols 0…14 with their extra bits, and
/// distance-0 self-extending copies. A stream produced with window
/// `w` decodes bit-exactly with any decoder window ≥ `w`
/// ([`decompress`] uses the 16384 default, which covers every level).
pub fn compress_lz(data: &[u8], window: usize) -> Result<Vec<u8>> {
    if !(1024..=16384).contains(&window) || !window.is_power_of_two() {
        return Err(Error::invalid(format!("GL window size {window}")));
    }
    Ok(write_tokens(&tokenize(data, window)))
}

/// Hash of the 3 bytes at `p` (caller guarantees they exist).
fn hash3(data: &[u8], p: usize) -> usize {
    const HASH_BITS: u32 = 15;
    let v = (u32::from(data[p]) << 16) | (u32::from(data[p + 1]) << 8) | u32::from(data[p + 2]);
    (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
}

/// Greedy hash-chain LZSS tokenizer. Chains are walked most-recent
/// first, so among equally long candidates the smallest distance
/// wins; a candidate farther back than the window ends the walk
/// (older entries are farther still).
fn tokenize(data: &[u8], window: usize) -> Vec<Token> {
    /// Longest hash chain walked per position (speed/ratio knob; has
    /// no effect on correctness).
    const CHAIN_LIMIT: usize = 128;
    let max_dist = window - 1;
    let mut head = vec![usize::MAX; 1 << 15];
    let mut prev = vec![usize::MAX; data.len()];
    let mut tokens = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if pos + MIN_MATCH <= data.len() {
            let mut cand = head[hash3(data, pos)];
            let max_len = (data.len() - pos).min(MAX_MATCH);
            let mut steps = 0usize;
            while cand != usize::MAX && steps < CHAIN_LIMIT {
                let dist = pos - cand - 1;
                if dist > max_dist {
                    break;
                }
                // Elementwise compare against the *input* is exact
                // even when the match overlaps its own output: the
                // decoder's byte-at-a-time copy reproduces exactly
                // that sequence.
                let mut l = 0usize;
                while l < max_len && data[cand + l] == data[pos + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = dist;
                    if l == max_len {
                        break;
                    }
                }
                cand = prev[cand];
                steps += 1;
            }
        }
        let insert = |head: &mut [usize], prev: &mut [usize], p: usize| {
            if p + MIN_MATCH <= data.len() {
                let h = hash3(data, p);
                prev[p] = head[h];
                head[h] = p;
            }
        };
        if best_len >= MIN_MATCH {
            tokens.push(Token::Match {
                len: best_len as u16,
                dist: best_dist as u16,
            });
            for k in 0..best_len {
                insert(&mut head, &mut prev, pos + k);
            }
            pos += best_len;
        } else {
            tokens.push(Token::Lit(data[pos]));
            insert(&mut head, &mut prev, pos);
            pos += 1;
        }
    }
    tokens
}

/// Per-symbol `(code, length)` table for a canonical length set
/// (length 0 = unused symbol; its entry must not be written).
fn codes_from_lengths(lengths: &[u8]) -> Vec<(u32, u8)> {
    let mut count = [0u32; MAX_CODE_LEN as usize + 1];
    for &l in lengths {
        if l > 0 {
            count[l as usize] += 1;
        }
    }
    let mut next_code = [0u32; MAX_CODE_LEN as usize + 1];
    let mut code = 0u32;
    for l in 1..=MAX_CODE_LEN as usize {
        next_code[l] = code;
        code = (code + count[l]) << 1;
    }
    lengths
        .iter()
        .map(|&l| {
            if l == 0 {
                (0, 0)
            } else {
                let c = next_code[l as usize];
                next_code[l as usize] += 1;
                (c, l)
            }
        })
        .collect()
}

/// Test helper: writes one symbol by walking its canonical code set
/// (slow linear scan — the production encoder uses
/// [`codes_from_lengths`] tables).
#[cfg(test)]
fn encode_symbol(w: &mut BitWriter, set: &CodeSet, sym: u32) {
    let CodeSet::Canonical(c) = set else {
        return; // degenerate sets encode nothing
    };
    for l in 1..=MAX_CODE_LEN as usize {
        if c.count[l] == 0 {
            continue;
        }
        let start = c.first_index[l] as usize;
        let end = start + c.count[l] as usize;
        if let Some(rel) = c.symbols[start..end].iter().position(|&s| s as u32 == sym) {
            w.write(c.first_code[l] + rel as u32, l as u32);
            return;
        }
    }
    unreachable!("symbol {sym} missing from its own code set");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let packed = compress(data);
        let (got, used) = decompress(&packed, data.len().max(16)).unwrap();
        assert_eq!(got, data);
        assert_eq!(used, packed.len(), "stream must end exactly on EOS");
    }

    #[test]
    fn roundtrip_empty() {
        roundtrip(&[]);
    }

    #[test]
    fn roundtrip_small() {
        roundtrip(b"abracadabra");
        roundtrip(&[0x80u8; 1000]);
        roundtrip(&[0u8, 255, 1, 254, 127, 128]);
    }

    #[test]
    fn roundtrip_all_byte_values() {
        let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        roundtrip(&data);
    }

    #[test]
    fn roundtrip_multi_block() {
        // > BLOCK_CODES bytes forces the multi-block continuation rule.
        let data: Vec<u8> = (0..(BLOCK_CODES + 1234))
            .map(|i| (i * 31 % 251) as u8)
            .collect();
        roundtrip(&data);
    }

    #[test]
    fn roundtrip_skewed_distribution() {
        // Long zero runs in the code-length lists exercise all three
        // run escapes of the literal-length reader.
        let mut data = vec![7u8; 4000];
        data.extend_from_slice(&[200u8; 100]);
        data.push(0);
        roundtrip(&data);
    }

    /// Reference expansion of a token stream — the oracle the
    /// documented copy rule must agree with. Linear output, so the
    /// self-extending overlap case falls out naturally.
    fn apply_tokens(tokens: &[Token]) -> Vec<u8> {
        let mut out = Vec::new();
        for t in tokens {
            match *t {
                Token::Lit(b) => out.push(b),
                Token::Match { len, dist } => {
                    for src in (out.len() - dist as usize - 1..).take(len as usize) {
                        let b = out[src];
                        out.push(b);
                    }
                }
            }
        }
        out
    }

    fn lz_roundtrip(data: &[u8], window: usize) {
        let packed = compress_lz(data, window).unwrap();
        let (got, used) = decompress_with_window(&packed, window, data.len().max(16)).unwrap();
        assert_eq!(got, data);
        assert_eq!(used, packed.len(), "stream must end exactly on EOS");
        // Any decoder window ≥ the compressor's decodes the same
        // stream; the default (16384) covers every level.
        let (dflt, _) = decompress(&packed, data.len().max(16)).unwrap();
        assert_eq!(dflt, data);
    }

    #[test]
    fn lz_roundtrip_every_window_level() {
        let mut data: Vec<u8> = (0..6000u32).map(|i| (i % 7 * 37) as u8).collect();
        data.extend_from_slice(b"a design a design a design of repeats of repeats");
        data.extend(std::iter::repeat(0xAB).take(500));
        for w in [1024, 2048, 4096, 8192, 16384] {
            lz_roundtrip(&data, w);
        }
    }

    #[test]
    fn lz_roundtrip_edge_inputs() {
        lz_roundtrip(&[], 1024);
        lz_roundtrip(b"ab", 1024); // below the minimum match length
        lz_roundtrip(b"aaa", 1024); // exactly one minimum match candidate
        lz_roundtrip(&[0u8; 100_000], 16384); // one long self-extending run
    }

    #[test]
    fn lz_roundtrip_seeded_random_mixtures() {
        // Deterministic LCG mixtures of fresh bytes and spliced
        // repeats of earlier chunks, across all five window levels.
        let mut s = 0x1234_5678u32;
        let mut next = move || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            s
        };
        for trial in 0..15usize {
            let len = (next() % 30_000) as usize;
            let mut data: Vec<u8> = Vec::with_capacity(len);
            while data.len() < len {
                if next() % 3 == 0 && !data.is_empty() {
                    let start = next() as usize % data.len();
                    let n = ((next() % 300) as usize + 3).min(data.len() - start);
                    let chunk = data[start..start + n].to_vec();
                    data.extend_from_slice(&chunk);
                } else {
                    for _ in 0..(next() % 50 + 1) {
                        data.push((next() >> 11) as u8);
                    }
                }
            }
            data.truncate(len);
            lz_roundtrip(&data, 1024 << (trial % 5));
        }
    }

    #[test]
    fn lz_multi_block_with_matches() {
        // More tokens than one block holds: an incompressible LCG
        // head (one literal token per byte, > BLOCK_CODES of them)
        // followed by a repetitive tail whose matches land in the
        // continuation blocks.
        let mut s = 0x00C8_AF5Bu32;
        let mut data: Vec<u8> = (0..(BLOCK_CODES + 2000))
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 17) as u8
            })
            .collect();
        for _ in 0..500 {
            data.extend_from_slice(b"needle thread needle thread");
        }
        lz_roundtrip(&data, 16384);
    }

    #[test]
    fn lz_beats_literal_only_on_repetitive_data() {
        let data: Vec<u8> = b"abcdefgh".iter().cycle().take(8192).copied().collect();
        let lz = compress_lz(&data, 16384).unwrap();
        let lit = compress(&data);
        assert!(
            lz.len() * 4 < lit.len(),
            "match layer unused: {} vs {} bytes",
            lz.len(),
            lit.len()
        );
    }

    #[test]
    fn lz_bad_window_rejected() {
        assert!(compress_lz(&[1, 2, 3], 512).is_err());
        assert!(compress_lz(&[1, 2, 3], 3000).is_err());
        assert!(compress_lz(&[1, 2, 3], 32768).is_err());
    }

    /// Every offset symbol 0…14, at both edges of its distance range,
    /// plus the extreme match lengths (3 and 256), through the real
    /// encoder tables and back through the decoder.
    #[test]
    fn every_offset_symbol_roundtrips() {
        let mut dists: Vec<u16> = vec![0, 1];
        for j in 2..=14u32 {
            dists.push(1 << (j - 1)); // low edge: extra bits all zero
            dists.push((1 << j) - 1); // high edge: extra bits all one
        }
        // A literal ramp long enough that every distance points at
        // real history.
        let mut tokens: Vec<Token> = (0..16384u32)
            .map(|i| Token::Lit((i.wrapping_mul(131) % 251) as u8))
            .collect();
        for &dist in &dists {
            tokens.push(Token::Match { len: 5, dist });
        }
        tokens.push(Token::Match {
            len: 256,
            dist: 300,
        }); // longest code (509)
        tokens.push(Token::Match {
            len: 3,
            dist: 16383,
        }); // shortest code, farthest reach
        let expect = apply_tokens(&tokens);
        let packed = write_tokens(&tokens);
        let (got, used) = decompress(&packed, expect.len()).unwrap();
        assert_eq!(got, expect);
        assert_eq!(used, packed.len());
    }

    /// Distance 0 with the maximum length: one literal fans out into
    /// a 257-byte run through the self-extending copy rule.
    #[test]
    fn self_extending_max_length_match() {
        let tokens = vec![Token::Lit(0x42), Token::Match { len: 256, dist: 0 }];
        let expect = apply_tokens(&tokens);
        assert_eq!(expect, vec![0x42; 257]);
        let (got, _) = decompress(&write_tokens(&tokens), 300).unwrap();
        assert_eq!(got, expect);
    }

    /// A match that reads across the ring-buffer wrap point of a
    /// small window.
    #[test]
    fn ring_wraparound_match() {
        // 2000 literals into a 1024-byte window: the ring has wrapped
        // almost twice before a full-window-reach match is decoded.
        let mut tokens: Vec<Token> = (0..2000u32).map(|i| Token::Lit((i % 199) as u8)).collect();
        tokens.push(Token::Match {
            len: 64,
            dist: 1023,
        });
        let expect = apply_tokens(&tokens);
        let packed = write_tokens(&tokens);
        let (got, _) = decompress_with_window(&packed, 1024, expect.len()).unwrap();
        assert_eq!(got, expect);
    }

    /// The tokenizer must never emit a match reaching outside its
    /// window; decoding under the *same* window proves it (a
    /// too-far distance would wrap to the wrong ring bytes).
    #[test]
    fn tokenizer_respects_small_window() {
        // Repeats spaced wider than the 1024 window: matchable under
        // 16384, out of reach under 1024.
        let mut data = Vec::new();
        for block in 0..8u8 {
            data.extend_from_slice(&[block; 40]);
            data.extend((0..1500u32).map(|i| (i % 256) as u8));
            data.extend_from_slice(b"a far-apart repeated phrase");
        }
        for t in tokenize(&data, 1024) {
            if let Token::Match { dist, .. } = t {
                assert!((dist as usize) < 1024);
            }
        }
        lz_roundtrip(&data, 1024);
        lz_roundtrip(&data, 16384);
    }

    /// A hand-built stream exercising the match and offset paths the
    /// literal-only encoder never emits: literal 'a', then a match of
    /// length 3 at distance 0 (self-extending run), then EOS.
    #[test]
    fn match_and_offset_decode() {
        let mut w = BitWriter::new();
        // Symbols: 'a' (97), match code 256 (length 3), EOS (510).
        let mut lit_lengths = vec![0u8; LIT_ALPHABET];
        lit_lengths[97] = 1;
        lit_lengths[256] = 2;
        lit_lengths[EOS] = 2;
        let lit = CodeSet::from_lengths(&lit_lengths).unwrap();
        // Offset set: the single symbol 0 (distance 0), length 1.
        let mut off_lengths = vec![0u8; OFF_ALPHABET];
        off_lengths[0] = 1;
        let off = CodeSet::from_lengths(&off_lengths).unwrap();
        // 19-set: lengths for the transmission below. We transmit the
        // literal lengths explicitly: symbol 3 (=length 1), zeros,
        // symbol 4 (=length 2). Runs: 0(×97? via escapes)…
        // Simpler: use a degenerate-free small set covering {0,1,2,3,4}.
        let mut small_lengths = vec![0u8; LEN_ALPHABET];
        for l in small_lengths.iter_mut().take(5) {
            *l = 3;
        }
        let small = CodeSet::from_lengths(&small_lengths).unwrap();

        w.write(3, 16); // three codes in the block
                        // 19-set: count 5, lengths 3,3,3,(2-bit zero field),3,3.
        w.write(5, 5);
        write_small_len(&mut w, 3);
        write_small_len(&mut w, 3);
        write_small_len(&mut w, 3);
        w.write(0, 2);
        write_small_len(&mut w, 3);
        write_small_len(&mut w, 3);
        // Literal lengths, count 511.
        w.write(LIT_ALPHABET as u32, 9);
        // 97 zeros: run 20+77 → symbol 2 with 9-bit 77.
        encode_symbol(&mut w, &small, 2);
        w.write(77, 9);
        // length 1 for 'a' → symbol 3.
        encode_symbol(&mut w, &small, 3);
        // zeros for 98..=255: 158 → symbol 2 with 9-bit 138.
        encode_symbol(&mut w, &small, 2);
        w.write(138, 9);
        // length 2 for 256 → symbol 4.
        encode_symbol(&mut w, &small, 4);
        // zeros for 257..=509: 253 → symbol 2 with 9-bit 233.
        encode_symbol(&mut w, &small, 2);
        w.write(233, 9);
        // length 2 for 510 → symbol 4.
        encode_symbol(&mut w, &small, 4);
        // Offset set: count 1, length 1 for symbol 0.
        w.write(1, 5);
        write_small_len(&mut w, 1);
        // Codes: 'a', match(len 3), EOS. Distance-0 copies the byte
        // just written, self-extending: "a" + "aaa".
        encode_symbol(&mut w, &lit, 97);
        encode_symbol(&mut w, &lit, 256);
        off.decode(&mut BitReader::new(&[0])).unwrap(); // sanity: off set decodes
        encode_symbol(&mut w, &off, 0);
        encode_symbol(&mut w, &lit, EOS as u32);
        let stream = w.finish();

        let (out, _) = decompress(&stream, 64).unwrap();
        assert_eq!(out, b"aaaa");
    }

    /// A match at distance 1 copies the byte before last.
    #[test]
    fn match_distance_one() {
        let mut w = BitWriter::new();
        let mut lit_lengths = vec![0u8; LIT_ALPHABET];
        lit_lengths[b'x' as usize] = 2;
        lit_lengths[b'y' as usize] = 2;
        lit_lengths[257] = 2; // match length 4
        lit_lengths[EOS] = 2;
        let lit = CodeSet::from_lengths(&lit_lengths).unwrap();
        let mut off_lengths = vec![0u8; OFF_ALPHABET];
        off_lengths[1] = 1; // distance = 2^0 + 0 extra bits… j=1 → 1 + 0-bit
        let off = CodeSet::from_lengths(&off_lengths).unwrap();
        let mut small_lengths = vec![0u8; LEN_ALPHABET];
        for l in small_lengths.iter_mut().take(7) {
            *l = 3;
        }
        let small = CodeSet::from_lengths(&small_lengths).unwrap();

        w.write(4, 16);
        w.write(7, 5);
        write_small_len(&mut w, 3);
        write_small_len(&mut w, 3);
        write_small_len(&mut w, 3);
        w.write(0, 2);
        for _ in 3..7 {
            write_small_len(&mut w, 3);
        }
        w.write(LIT_ALPHABET as u32, 9);
        // zeros to 'x' (120): 120 → 20+100 (sym 2, 9-bit 100).
        encode_symbol(&mut w, &small, 2);
        w.write(100, 9);
        encode_symbol(&mut w, &small, 4); // len 2 for 'x'
        encode_symbol(&mut w, &small, 4); // len 2 for 'y' (121)
                                          // zeros 122..=256: 135 → sym 2, 9-bit 115.
        encode_symbol(&mut w, &small, 2);
        w.write(115, 9);
        encode_symbol(&mut w, &small, 4); // len 2 for 257
                                          // zeros 258..=509: 252 → sym 2, 9-bit 232.
        encode_symbol(&mut w, &small, 2);
        w.write(232, 9);
        encode_symbol(&mut w, &small, 4); // len 2 for EOS
                                          // Offset set: count 2 → lengths 0, 1.
        w.write(2, 5);
        write_small_len(&mut w, 0);
        write_small_len(&mut w, 1);
        // 'x', 'y', match(len 4, j=1 → distance 1 + 0 extra bits).
        encode_symbol(&mut w, &lit, b'x' as u32);
        encode_symbol(&mut w, &lit, b'y' as u32);
        encode_symbol(&mut w, &lit, 257);
        encode_symbol(&mut w, &off, 1);
        // j=1 → read j−1 = 0 extra bits; distance = 1.
        encode_symbol(&mut w, &lit, EOS as u32);
        let stream = w.finish();

        let (out, _) = decompress(&stream, 64).unwrap();
        // Source = pos − 1 − 1: alternating copy of "xy".
        assert_eq!(out, b"xyxyxy");
    }

    #[test]
    fn degenerate_literal_set_decodes_empty() {
        let mut w = BitWriter::new();
        w.write(1, 16); // one code
        w.write(0, 5); // degenerate 19-set…
        w.write(0, 5); // …yielding symbol 0
        w.write(0, 9); // degenerate literal set…
        w.write(EOS as u32, 9); // …every lookup yields EOS
        w.write(0, 5); // degenerate offset set
        w.write(0, 5);
        let stream = w.finish();
        let (out, _) = decompress(&stream, 16).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn output_limit_enforced() {
        let packed = compress(&[7u8; 100]);
        assert!(matches!(
            decompress(&packed, 10),
            Err(Error::Invalid { .. })
        ));
    }

    #[test]
    fn truncated_stream_errors() {
        let packed = compress(b"hello world hello world");
        assert!(matches!(
            decompress(&packed[..packed.len() - 2], 64),
            Err(Error::UnexpectedEof { .. }) | Err(Error::Invalid { .. })
        ));
    }

    #[test]
    fn bad_window_rejected() {
        assert!(decompress_with_window(&[0; 8], 512, 16).is_err());
        assert!(decompress_with_window(&[0; 8], 3000, 16).is_err());
    }
}
