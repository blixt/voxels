# Pre-upgrade Brotli compatibility fixtures

These binary fixtures were generated on 2026-09-12 from the unmodified `world/src` at commit
`27b558f55461681d2e35ae35fd4695d73d2716e9`, using a separate temporary Cargo project with
`brotli = "=8.0.4"` (and `brotli-decompressor = "5.0.3"`), Rust 1.98.1, and no user world data.
The copied crate used blake3 1.8.7, bytemuck 1.25.2, and serde 1.0.229. Generation used two build jobs
and a separate target directory. They intentionally remain encoded with the prior dependency.

`terrain-page.brotli8.vxtp` is the output of `encode_terrain_page`: an exact level-0 clustered
page at `[-2, 1, 7]`, source hash `[42; 32]`, and revision 93. Its material sampler is:

```rust
if coord.y < 37 { Material::Stone }
else if coord.y == 37 { Material::Dirt }
else if coord.y == 38 && coord.x.rem_euclid(8) == 0 { Material::Wood }
else if coord.y == 39 && coord.x.rem_euclid(8) <= 2 { Material::Leaves }
else if coord.y == 38 && coord.z.rem_euclid(8) < 2 { Material::Water }
else { Material::Air }
```

`terrain-page-result.brotli8.vxwp` is the output of `encode_virtual_terrain_page_batch_result`,
containing that same page as the sole successful item, its matching transfer identity, source hash
`[42; 32]`, and request ID 73. This freezes both compression layers: VXTP uses quality 5 / window 20;
the VXWP result envelope uses quality 2 / window 20. The tests decode through the public versioned
codecs and compare canonical semantic content; they do not require newer encoders to reproduce the
same compressed bytes.

The canonical page fingerprint is
`1c8202218a35ae22d41a5f9e11c7a822af050fb980cb14401f2df6530db470b9`.

The adjacent world-service test `signed_session_cryptography_matches_the_webcrypto_golden_vector`
freezes HMAC-SHA-256 and URL-safe unpadded Base64 interoperability independently of RustCrypto.
It was generated with Node 26.7.0 WebCrypto using the same `importKey` and `sign` calls as
`cloudflare/session.ts`, the literal test-only key in the test, expiry `1800000000` (base36 `tro8w0`),
browser bytes `00..0f`, player bytes `f0..ff`, and nonce bytes `80..8b`. A separate Python
`hmac`/`hashlib` calculation confirmed the signature. The existing current-clock test continues to
exercise expiry and full token authorization; the fixed vector has no dependency on wall-clock time.
