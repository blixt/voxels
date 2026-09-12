use voxels_world::codec::encode_chunk;
use voxels_world::{
    CINDER_VAULT, Chunk, ChunkCoord, GENERATOR_VERSION, Generator, Material, ProceduralWorldSource,
    VoxelCoord, WorldProduct, WorldProductBatch, WorldProductPriority, WorldProductRequest,
    WorldSourceEngine,
};

const SEED: u64 = 0x5eed_cafe;
const REPRESENTATIVE_CHUNKS: [(&str, ChunkCoord); 5] = [
    ("ordinary", ChunkCoord::new(0, 0, 0)),
    ("pilgrim-road", ChunkCoord::new(-90, 1, 69)),
    ("water", ChunkCoord::new(563, 0, 403)),
    ("alpine-needle", ChunkCoord::new(-7, 1, -41)),
    ("cinder-vault", ChunkCoord::new(-162, 0, 103)),
];

fn voxel_hash(chunk: &Chunk) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"voxels-procedural-v17-chunk-golden-v1\0");
    for material in chunk.voxels() {
        hasher.update(&material.id().to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn generated_batch() -> Vec<Chunk> {
    let source = ProceduralWorldSource::new(SEED);
    let generated = source.generate_batch(WorldProductBatch {
        priority: WorldProductPriority::VisibleChunk,
        requests: REPRESENTATIVE_CHUNKS
            .into_iter()
            .map(|(_, coord)| WorldProductRequest::ChunkWithHalo(coord))
            .collect(),
    });
    let chunks = match generated {
        Ok(result) => result
            .items
            .into_iter()
            .filter_map(|item| match item.result {
                Ok(WorldProduct::Chunk(snapshot)) => Some(snapshot.chunk),
                Ok(_) | Err(_) => None,
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            assert_eq!(
                Some(error),
                None,
                "procedural-v17 representative batch must succeed"
            );
            Vec::new()
        }
    };
    assert_eq!(chunks.len(), REPRESENTATIVE_CHUNKS.len());
    chunks
}

#[test]
fn procedural_v17_representative_chunks_keep_their_canonical_voxels() {
    assert_eq!(GENERATOR_VERSION, 17);
    assert_eq!(Material::SCHEMA_VERSION, 3);
    assert_eq!(
        VoxelCoord::new(
            CINDER_VAULT.chamber[0],
            CINDER_VAULT.chamber[1],
            CINDER_VAULT.chamber[2]
        )
        .chunk(),
        REPRESENTATIVE_CHUNKS[4].1
    );

    let chunks = generated_batch();
    let ordinary = &chunks[0];
    assert!(
        ordinary
            .voxels()
            .iter()
            .any(|material| material.is_collidable())
    );
    let actual = chunks.iter().map(voxel_hash).collect::<Vec<_>>();
    assert_eq!(
        actual,
        [
            "ae72e8f49630a581e818611c78c8c2c3301f0522bef08ecfac140ff067832e18",
            "7b120a456f7d827d909869926af4949936cb7d1debb3b61e3c693216f7ae60ba",
            "9a178587cd3522d85534fece62739959b5f936f5c605372c2e1c4279ecb7deab",
            "d51723eea0d59566c99d7e1605f0f237627d5e05fe1ce37ecfa435d5dfdd205a",
            "cd396aa57a2cc14cce65acd3220df5f4b3881fbbc0e8225b4f294e024574c329",
        ]
    );

    let identity = ProceduralWorldSource::new(SEED).source_identity_hash();
    let encoded_sizes = chunks
        .iter()
        .map(|chunk| encode_chunk(chunk, identity).len())
        .collect::<Vec<_>>();
    assert_eq!(encoded_sizes, [106, 12_402, 4_204, 106, 8_304]);
}

#[test]
fn procedural_source_adapter_is_byte_identical_to_generator_v17() {
    let generator = Generator::new(SEED);
    for ((label, coord), adapted) in REPRESENTATIVE_CHUNKS.into_iter().zip(generated_batch()) {
        assert_eq!(
            adapted,
            generator.generate_chunk(coord),
            "adapter changed {label}"
        );
    }
}
