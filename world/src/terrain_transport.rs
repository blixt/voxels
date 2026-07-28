//! Strict wire framing for virtual terrain page requests and results.
//!
//! Directory nodes provide immutable `(key, revision, fingerprint)` identities. Transfers repeat
//! that identity and successful results are decoded before acceptance, so a stale or mislabeled
//! payload cannot enter a resident hierarchy.

use crate::{
    TERRAIN_PAGE_TARGET_COMPRESSED_BYTES, TerrainPageCodecError, TerrainPageKey, TerrainPageV1,
    WorldSourceIdentityHash, decode_terrain_page, encode_terrain_page,
};
use std::fmt;

pub const TERRAIN_PAGE_TRANSFER_SCHEMA_VERSION: u16 = 2;
pub const TERRAIN_PAGE_TRANSFER_MAX_ITEMS: usize = 256;
pub const TERRAIN_PAGE_TRANSFER_MAX_BYTES: usize =
    TERRAIN_PAGE_TRANSFER_MAX_ITEMS * TERRAIN_PAGE_TARGET_COMPRESSED_BYTES + 32_768;
const REQUEST_MAGIC: &[u8; 4] = b"VXPR";
const RESULT_MAGIC: &[u8; 4] = b"VXPS";
const HEADER_BYTES: usize = 80;
const REQUEST_ITEM_BYTES: usize = 56;
const RESULT_ITEM_HEADER_BYTES: usize = 64;
const REQUEST_HASH_DOMAIN: &[u8] = b"voxels-terrain-page-request-v2\0";
const RESULT_HASH_DOMAIN: &[u8] = b"voxels-terrain-page-result-v2\0";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TerrainPageTransferIdentity {
    pub key: TerrainPageKey,
    pub revision: u64,
    pub content_fingerprint: [u8; 32],
}

impl TerrainPageTransferIdentity {
    pub fn matches(self, page: &TerrainPageV1) -> bool {
        self.key == page.key
            && self.revision == page.revision
            && self.content_fingerprint == page.content_fingerprint
    }

    fn validates(self) -> bool {
        (if self.key.is_surface() {
            self.key.horizontal_bounds().is_some()
        } else {
            self.key.bounds().is_some()
        }) && self.content_fingerprint != [0; 32]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainPageBatchRequestV1 {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub pages: Vec<TerrainPageTransferIdentity>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainPageTransferFailure {
    Unavailable = 1,
    StaleRevision = 2,
    GenerationFailed = 3,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainPageBatchItemV1 {
    pub requested: TerrainPageTransferIdentity,
    pub result: Result<TerrainPageV1, TerrainPageTransferFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainPageBatchResultV1 {
    pub source_identity_hash: WorldSourceIdentityHash,
    pub items: Vec<TerrainPageBatchItemV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerrainPageTransferCodecError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeader(&'static str),
    SourceMismatch,
    InvalidIdentity,
    DuplicateOrUnsortedIdentity,
    UnknownFailure(u8),
    PageIdentityMismatch,
    LimitExceeded(&'static str),
    CorruptHash,
    Page(TerrainPageCodecError),
}

impl fmt::Display for TerrainPageTransferCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("truncated virtual terrain page transfer"),
            Self::InvalidMagic => formatter.write_str("invalid virtual terrain transfer magic"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported virtual terrain transfer version {version}"
                )
            }
            Self::InvalidHeader(reason) => {
                write!(
                    formatter,
                    "invalid virtual terrain transfer header: {reason}"
                )
            }
            Self::SourceMismatch => formatter.write_str("virtual terrain transfer source mismatch"),
            Self::InvalidIdentity => {
                formatter.write_str("virtual terrain transfer contains an invalid page identity")
            }
            Self::DuplicateOrUnsortedIdentity => formatter
                .write_str("virtual terrain transfer page identities are duplicate or unsorted"),
            Self::UnknownFailure(value) => {
                write!(
                    formatter,
                    "unknown virtual terrain transfer failure {value}"
                )
            }
            Self::PageIdentityMismatch => {
                formatter.write_str("virtual terrain payload does not match its requested identity")
            }
            Self::LimitExceeded(limit) => {
                write!(
                    formatter,
                    "virtual terrain transfer limit exceeded: {limit}"
                )
            }
            Self::CorruptHash => formatter.write_str("virtual terrain transfer hash mismatch"),
            Self::Page(error) => write!(formatter, "virtual terrain page is invalid: {error}"),
        }
    }
}

impl std::error::Error for TerrainPageTransferCodecError {}

impl From<TerrainPageCodecError> for TerrainPageTransferCodecError {
    fn from(error: TerrainPageCodecError) -> Self {
        Self::Page(error)
    }
}

pub fn encode_terrain_page_batch_request(
    request: &TerrainPageBatchRequestV1,
) -> Result<Vec<u8>, TerrainPageTransferCodecError> {
    validate_identities(&request.pages)?;
    let mut body = Vec::with_capacity(request.pages.len() * REQUEST_ITEM_BYTES);
    for identity in &request.pages {
        encode_identity(&mut body, *identity);
    }
    encode_envelope(
        REQUEST_MAGIC,
        REQUEST_HASH_DOMAIN,
        request.source_identity_hash,
        request.pages.len(),
        &body,
    )
}

pub fn decode_terrain_page_batch_request(
    bytes: &[u8],
    expected_source: WorldSourceIdentityHash,
) -> Result<TerrainPageBatchRequestV1, TerrainPageTransferCodecError> {
    let envelope = decode_envelope(bytes, REQUEST_MAGIC, REQUEST_HASH_DOMAIN, expected_source)?;
    let expected_body_bytes = envelope.item_count.checked_mul(REQUEST_ITEM_BYTES).ok_or(
        TerrainPageTransferCodecError::LimitExceeded("request body bytes"),
    )?;
    if envelope.body.len() != expected_body_bytes {
        return Err(TerrainPageTransferCodecError::InvalidHeader(
            "request body length",
        ));
    }
    let mut cursor = Cursor::new(envelope.body);
    let mut pages = Vec::with_capacity(envelope.item_count);
    for _ in 0..envelope.item_count {
        pages.push(decode_identity(&mut cursor)?);
    }
    validate_identities(&pages)?;
    Ok(TerrainPageBatchRequestV1 {
        source_identity_hash: expected_source,
        pages,
    })
}

pub fn encode_terrain_page_batch_result(
    result: &TerrainPageBatchResultV1,
) -> Result<Vec<u8>, TerrainPageTransferCodecError> {
    validate_result_items(&result.items, result.source_identity_hash)?;
    let mut body = Vec::new();
    for item in &result.items {
        encode_identity(&mut body, item.requested);
        match &item.result {
            Ok(page) => {
                let encoded = encode_terrain_page(page)?;
                if encoded.len() > TERRAIN_PAGE_TARGET_COMPRESSED_BYTES {
                    return Err(TerrainPageTransferCodecError::LimitExceeded(
                        "published page bytes",
                    ));
                }
                body.push(0);
                body.extend_from_slice(&[0; 3]);
                push_u32(&mut body, encoded.len() as u32);
                body.extend_from_slice(&encoded);
            }
            Err(failure) => {
                body.push(*failure as u8);
                body.extend_from_slice(&[0; 3]);
                push_u32(&mut body, 0);
            }
        }
        if body.len() > TERRAIN_PAGE_TRANSFER_MAX_BYTES {
            return Err(TerrainPageTransferCodecError::LimitExceeded(
                "result body bytes",
            ));
        }
    }
    encode_envelope(
        RESULT_MAGIC,
        RESULT_HASH_DOMAIN,
        result.source_identity_hash,
        result.items.len(),
        &body,
    )
}

pub fn decode_terrain_page_batch_result(
    bytes: &[u8],
    expected_source: WorldSourceIdentityHash,
) -> Result<TerrainPageBatchResultV1, TerrainPageTransferCodecError> {
    let envelope = decode_envelope(bytes, RESULT_MAGIC, RESULT_HASH_DOMAIN, expected_source)?;
    let mut cursor = Cursor::new(envelope.body);
    let mut items = Vec::with_capacity(envelope.item_count);
    for _ in 0..envelope.item_count {
        if cursor.remaining() < RESULT_ITEM_HEADER_BYTES {
            return Err(TerrainPageTransferCodecError::Truncated);
        }
        let requested = decode_identity(&mut cursor)?;
        let status = cursor.u8()?;
        if cursor.take(3)? != [0; 3] {
            return Err(TerrainPageTransferCodecError::InvalidHeader(
                "result item reserved bytes",
            ));
        }
        let payload_bytes = usize::try_from(cursor.u32()?).map_err(|_| {
            TerrainPageTransferCodecError::LimitExceeded("result item payload bytes")
        })?;
        if payload_bytes > TERRAIN_PAGE_TARGET_COMPRESSED_BYTES {
            return Err(TerrainPageTransferCodecError::LimitExceeded(
                "published page bytes",
            ));
        }
        let payload = cursor.take(payload_bytes)?;
        let result = if status == 0 {
            if payload.is_empty() {
                return Err(TerrainPageTransferCodecError::InvalidHeader(
                    "successful result has no page",
                ));
            }
            let page = decode_terrain_page(payload, expected_source)?;
            if !requested.matches(&page) {
                return Err(TerrainPageTransferCodecError::PageIdentityMismatch);
            }
            Ok(page)
        } else {
            if !payload.is_empty() {
                return Err(TerrainPageTransferCodecError::InvalidHeader(
                    "failed result has a payload",
                ));
            }
            Err(match status {
                1 => TerrainPageTransferFailure::Unavailable,
                2 => TerrainPageTransferFailure::StaleRevision,
                3 => TerrainPageTransferFailure::GenerationFailed,
                value => return Err(TerrainPageTransferCodecError::UnknownFailure(value)),
            })
        };
        items.push(TerrainPageBatchItemV1 { requested, result });
    }
    if cursor.remaining() != 0 {
        return Err(TerrainPageTransferCodecError::InvalidHeader(
            "result trailing bytes",
        ));
    }
    validate_result_items(&items, expected_source)?;
    Ok(TerrainPageBatchResultV1 {
        source_identity_hash: expected_source,
        items,
    })
}

fn validate_identities(
    identities: &[TerrainPageTransferIdentity],
) -> Result<(), TerrainPageTransferCodecError> {
    if identities.is_empty() || identities.len() > TERRAIN_PAGE_TRANSFER_MAX_ITEMS {
        return Err(TerrainPageTransferCodecError::LimitExceeded(
            "page identity count",
        ));
    }
    if identities.iter().any(|identity| !identity.validates()) {
        return Err(TerrainPageTransferCodecError::InvalidIdentity);
    }
    if !identities.windows(2).all(|pair| pair[0].key < pair[1].key) {
        return Err(TerrainPageTransferCodecError::DuplicateOrUnsortedIdentity);
    }
    Ok(())
}

fn validate_result_items(
    items: &[TerrainPageBatchItemV1],
    source: WorldSourceIdentityHash,
) -> Result<(), TerrainPageTransferCodecError> {
    let identities = items.iter().map(|item| item.requested).collect::<Vec<_>>();
    validate_identities(&identities)?;
    for item in items {
        if let Ok(page) = &item.result {
            if page.source_identity_hash != source {
                return Err(TerrainPageTransferCodecError::SourceMismatch);
            }
            if !item.requested.matches(page) {
                return Err(TerrainPageTransferCodecError::PageIdentityMismatch);
            }
        }
    }
    Ok(())
}

fn encode_envelope(
    magic: &[u8; 4],
    domain: &[u8],
    source: WorldSourceIdentityHash,
    item_count: usize,
    body: &[u8],
) -> Result<Vec<u8>, TerrainPageTransferCodecError> {
    if item_count == 0 || item_count > TERRAIN_PAGE_TRANSFER_MAX_ITEMS {
        return Err(TerrainPageTransferCodecError::LimitExceeded("item count"));
    }
    if body.len() > TERRAIN_PAGE_TRANSFER_MAX_BYTES {
        return Err(TerrainPageTransferCodecError::LimitExceeded("body bytes"));
    }
    let item_count = u16::try_from(item_count)
        .map_err(|_| TerrainPageTransferCodecError::LimitExceeded("item count"))?;
    let body_bytes = u32::try_from(body.len())
        .map_err(|_| TerrainPageTransferCodecError::LimitExceeded("body bytes"))?;
    let hash = envelope_hash(domain, source, item_count, body);
    let mut encoded = Vec::with_capacity(HEADER_BYTES + body.len());
    encoded.extend_from_slice(magic);
    push_u16(&mut encoded, TERRAIN_PAGE_TRANSFER_SCHEMA_VERSION);
    push_u16(&mut encoded, HEADER_BYTES as u16);
    encoded.extend_from_slice(source.as_bytes());
    push_u16(&mut encoded, item_count);
    push_u16(&mut encoded, 0);
    push_u32(&mut encoded, body_bytes);
    encoded.extend_from_slice(&hash);
    debug_assert_eq!(encoded.len(), HEADER_BYTES);
    encoded.extend_from_slice(body);
    Ok(encoded)
}

struct Envelope<'a> {
    item_count: usize,
    body: &'a [u8],
}

fn decode_envelope<'a>(
    bytes: &'a [u8],
    magic: &[u8; 4],
    domain: &[u8],
    expected_source: WorldSourceIdentityHash,
) -> Result<Envelope<'a>, TerrainPageTransferCodecError> {
    if bytes.len() < HEADER_BYTES {
        return Err(TerrainPageTransferCodecError::Truncated);
    }
    if bytes.len() > HEADER_BYTES + TERRAIN_PAGE_TRANSFER_MAX_BYTES {
        return Err(TerrainPageTransferCodecError::LimitExceeded(
            "encoded bytes",
        ));
    }
    let mut cursor = Cursor::new(bytes);
    if cursor.take(4)? != magic {
        return Err(TerrainPageTransferCodecError::InvalidMagic);
    }
    let version = cursor.u16()?;
    if version != TERRAIN_PAGE_TRANSFER_SCHEMA_VERSION {
        return Err(TerrainPageTransferCodecError::UnsupportedVersion(version));
    }
    if usize::from(cursor.u16()?) != HEADER_BYTES {
        return Err(TerrainPageTransferCodecError::InvalidHeader("header bytes"));
    }
    let source = WorldSourceIdentityHash::from_bytes(cursor.array()?);
    if source != expected_source {
        return Err(TerrainPageTransferCodecError::SourceMismatch);
    }
    let item_count = usize::from(cursor.u16()?);
    if item_count == 0 || item_count > TERRAIN_PAGE_TRANSFER_MAX_ITEMS {
        return Err(TerrainPageTransferCodecError::LimitExceeded("item count"));
    }
    if cursor.u16()? != 0 {
        return Err(TerrainPageTransferCodecError::InvalidHeader(
            "reserved bytes",
        ));
    }
    let body_bytes = usize::try_from(cursor.u32()?)
        .map_err(|_| TerrainPageTransferCodecError::LimitExceeded("body bytes"))?;
    if body_bytes > TERRAIN_PAGE_TRANSFER_MAX_BYTES {
        return Err(TerrainPageTransferCodecError::LimitExceeded("body bytes"));
    }
    let claimed_hash = cursor.array::<32>()?;
    let body = cursor.take(body_bytes)?;
    if cursor.remaining() != 0 {
        return Err(TerrainPageTransferCodecError::InvalidHeader(
            "envelope trailing bytes",
        ));
    }
    if claimed_hash != envelope_hash(domain, source, item_count as u16, body) {
        return Err(TerrainPageTransferCodecError::CorruptHash);
    }
    Ok(Envelope { item_count, body })
}

fn envelope_hash(
    domain: &[u8],
    source: WorldSourceIdentityHash,
    item_count: u16,
    body: &[u8],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&TERRAIN_PAGE_TRANSFER_SCHEMA_VERSION.to_le_bytes());
    hasher.update(source.as_bytes());
    hasher.update(&item_count.to_le_bytes());
    hasher.update(&(body.len() as u32).to_le_bytes());
    hasher.update(body);
    *hasher.finalize().as_bytes()
}

fn encode_identity(bytes: &mut Vec<u8>, identity: TerrainPageTransferIdentity) {
    bytes.push(identity.key.level);
    bytes.extend_from_slice(&[0; 3]);
    for component in identity.key.coord {
        push_i32(bytes, component);
    }
    push_u64(bytes, identity.revision);
    bytes.extend_from_slice(&identity.content_fingerprint);
}

fn decode_identity(
    cursor: &mut Cursor<'_>,
) -> Result<TerrainPageTransferIdentity, TerrainPageTransferCodecError> {
    let level = cursor.u8()?;
    if cursor.take(3)? != [0; 3] {
        return Err(TerrainPageTransferCodecError::InvalidHeader(
            "identity reserved bytes",
        ));
    }
    let identity = TerrainPageTransferIdentity {
        key: TerrainPageKey {
            level,
            coord: [cursor.i32()?, cursor.i32()?, cursor.i32()?],
        },
        revision: cursor.u64()?,
        content_fingerprint: cursor.array()?,
    };
    if !identity.validates() {
        return Err(TerrainPageTransferCodecError::InvalidIdentity);
    }
    Ok(identity)
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], TerrainPageTransferCodecError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(TerrainPageTransferCodecError::Truncated)?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(TerrainPageTransferCodecError::Truncated)?;
        self.offset = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], TerrainPageTransferCodecError> {
        self.take(N)?
            .try_into()
            .map_err(|_| TerrainPageTransferCodecError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, TerrainPageTransferCodecError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, TerrainPageTransferCodecError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, TerrainPageTransferCodecError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, TerrainPageTransferCodecError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, TerrainPageTransferCodecError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Material, VoxelCoord, build_exact_terrain_page};

    fn source() -> WorldSourceIdentityHash {
        WorldSourceIdentityHash::from_bytes([0x6d; 32])
    }

    fn pages() -> Vec<TerrainPageV1> {
        [
            TerrainPageKey {
                level: 0,
                coord: [-2, -1, 3],
            },
            TerrainPageKey {
                level: 0,
                coord: [-1, -1, 3],
            },
        ]
        .into_iter()
        .enumerate()
        .map(|(index, key)| {
            build_exact_terrain_page(source(), key, index as u64 + 7, |coord: VoxelCoord| {
                if coord.y <= -1 {
                    Material::Stone
                } else {
                    Material::Air
                }
            })
            .unwrap()
        })
        .collect()
    }

    fn identity(page: &TerrainPageV1) -> TerrainPageTransferIdentity {
        TerrainPageTransferIdentity {
            key: page.key,
            revision: page.revision,
            content_fingerprint: page.content_fingerprint,
        }
    }

    #[test]
    fn request_and_mixed_result_round_trip_exact_page_identities() {
        let pages = pages();
        let identities = pages.iter().map(identity).collect::<Vec<_>>();
        let request = TerrainPageBatchRequestV1 {
            source_identity_hash: source(),
            pages: identities.clone(),
        };
        let encoded_request = encode_terrain_page_batch_request(&request).unwrap();
        assert_eq!(
            decode_terrain_page_batch_request(&encoded_request, source()).unwrap(),
            request
        );

        let result = TerrainPageBatchResultV1 {
            source_identity_hash: source(),
            items: vec![
                TerrainPageBatchItemV1 {
                    requested: identities[0],
                    result: Ok(pages[0].clone()),
                },
                TerrainPageBatchItemV1 {
                    requested: identities[1],
                    result: Err(TerrainPageTransferFailure::Unavailable),
                },
            ],
        };
        let encoded_result = encode_terrain_page_batch_result(&result).unwrap();
        assert_eq!(
            decode_terrain_page_batch_result(&encoded_result, source()).unwrap(),
            result
        );
    }

    #[test]
    fn request_round_trips_signed_surface_page_identity() {
        let request = TerrainPageBatchRequestV1 {
            source_identity_hash: source(),
            pages: vec![TerrainPageTransferIdentity {
                key: TerrainPageKey::surface(7, -3, 5),
                revision: 29,
                content_fingerprint: [0x6d; 32],
            }],
        };
        let encoded = encode_terrain_page_batch_request(&request).unwrap();
        assert_eq!(
            decode_terrain_page_batch_request(&encoded, source()).unwrap(),
            request
        );
    }

    #[test]
    fn transfer_rejects_unsorted_stale_corrupt_and_trailing_data() {
        let pages = pages();
        let mut identities = pages.iter().map(identity).collect::<Vec<_>>();
        identities.reverse();
        assert_eq!(
            encode_terrain_page_batch_request(&TerrainPageBatchRequestV1 {
                source_identity_hash: source(),
                pages: identities,
            }),
            Err(TerrainPageTransferCodecError::DuplicateOrUnsortedIdentity)
        );

        let mut stale = identity(&pages[0]);
        stale.revision += 1;
        assert_eq!(
            encode_terrain_page_batch_result(&TerrainPageBatchResultV1 {
                source_identity_hash: source(),
                items: vec![TerrainPageBatchItemV1 {
                    requested: stale,
                    result: Ok(pages[0].clone()),
                }],
            }),
            Err(TerrainPageTransferCodecError::PageIdentityMismatch)
        );

        let request = TerrainPageBatchRequestV1 {
            source_identity_hash: source(),
            pages: pages.iter().map(identity).collect(),
        };
        let mut corrupt = encode_terrain_page_batch_request(&request).unwrap();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x80;
        assert_eq!(
            decode_terrain_page_batch_request(&corrupt, source()),
            Err(TerrainPageTransferCodecError::CorruptHash)
        );
        let mut trailing = encode_terrain_page_batch_request(&request).unwrap();
        trailing.push(0);
        assert_eq!(
            decode_terrain_page_batch_request(&trailing, source()),
            Err(TerrainPageTransferCodecError::InvalidHeader(
                "envelope trailing bytes"
            ))
        );
    }
}
