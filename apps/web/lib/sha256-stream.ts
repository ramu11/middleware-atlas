// middleware-atlas: streaming SHA-256 for duplicate artifact detection
// WebCrypto's digest() requires a complete ArrayBuffer. This incremental implementation
// reads Blob streams in bounded chunks, so large heap dumps do not require multi-GB RAM.
const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);
const INITIAL = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
const rotr = (value: number, bits: number): number => (value >>> bits) | (value << (32 - bits));

class StreamingSha256 {
  private readonly state = Uint32Array.from(INITIAL);
  private readonly block = new Uint8Array(64);
  private readonly words = new Uint32Array(64);
  private blockLength = 0;
  private totalBytes = 0;

  update(bytes: Uint8Array): void {
    this.totalBytes += bytes.length;
    let offset = 0;
    if (this.blockLength > 0) {
      const take = Math.min(64 - this.blockLength, bytes.length);
      this.block.set(bytes.subarray(0, take), this.blockLength);
      this.blockLength += take;
      offset += take;
      if (this.blockLength === 64) {
        this.compress(this.block);
        this.blockLength = 0;
      }
    }
    while (offset + 64 <= bytes.length) {
      this.compress(bytes.subarray(offset, offset + 64));
      offset += 64;
    }
    if (offset < bytes.length) {
      this.block.set(bytes.subarray(offset), 0);
      this.blockLength = bytes.length - offset;
    }
  }

  digestHex(): string {
    const bitLength = this.totalBytes * 8;
    const tailLength = this.blockLength < 56 ? 64 : 128;
    const tail = new Uint8Array(tailLength);
    tail.set(this.block.subarray(0, this.blockLength));
    tail[this.blockLength] = 0x80;
    const view = new DataView(tail.buffer);
    view.setUint32(tailLength - 8, Math.floor(bitLength / 0x100000000), false);
    view.setUint32(tailLength - 4, bitLength >>> 0, false);
    for (let offset = 0; offset < tail.length; offset += 64) {
      this.compress(tail.subarray(offset, offset + 64));
    }
    return Array.from(this.state, (word) => word.toString(16).padStart(8, "0")).join("");
  }

  private compress(block: Uint8Array): void {
    const view = new DataView(block.buffer, block.byteOffset, block.byteLength);
    for (let index = 0; index < 16; index += 1) this.words[index] = view.getUint32(index * 4, false);
    for (let index = 16; index < 64; index += 1) {
      const x = this.words[index - 15];
      const y = this.words[index - 2];
      const s0 = rotr(x, 7) ^ rotr(x, 18) ^ (x >>> 3);
      const s1 = rotr(y, 17) ^ rotr(y, 19) ^ (y >>> 10);
      this.words[index] = (this.words[index - 16] + s0 + this.words[index - 7] + s1) >>> 0;
    }
    let a = this.state[0], b = this.state[1], c = this.state[2], d = this.state[3];
    let e = this.state[4], f = this.state[5], g = this.state[6], h = this.state[7];
    for (let index = 0; index < 64; index += 1) {
      const sum1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const choice = (e & f) ^ (~e & g);
      const temp1 = (h + sum1 + choice + K[index] + this.words[index]) >>> 0;
      const sum0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const majority = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (sum0 + majority) >>> 0;
      h = g; g = f; f = e; e = (d + temp1) >>> 0;
      d = c; c = b; b = a; a = (temp1 + temp2) >>> 0;
    }
    this.state[0] = (this.state[0] + a) >>> 0;
    this.state[1] = (this.state[1] + b) >>> 0;
    this.state[2] = (this.state[2] + c) >>> 0;
    this.state[3] = (this.state[3] + d) >>> 0;
    this.state[4] = (this.state[4] + e) >>> 0;
    this.state[5] = (this.state[5] + f) >>> 0;
    this.state[6] = (this.state[6] + g) >>> 0;
    this.state[7] = (this.state[7] + h) >>> 0;
  }
}

export async function sha256Blob(blob: Blob, onProgress?: (percent: number) => void): Promise<string> {
  const hasher = new StreamingSha256();
  const reader = blob.stream().getReader();
  let processed = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      hasher.update(value);
      processed += value.length;
      if (onProgress && blob.size > 0) onProgress(Math.min(100, Math.floor((processed / blob.size) * 100)));
    }
  } finally {
    reader.releaseLock();
  }
  onProgress?.(100);
  return hasher.digestHex();
}
