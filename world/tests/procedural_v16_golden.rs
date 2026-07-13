use voxels_world::codec::encode_chunk;
use voxels_world::{
    CINDER_VAULT, Chunk, ChunkCoord, GENERATOR_VERSION, Generator, Material, ProceduralWorldSource,
    VoxelCoord, WorldProduct, WorldProductBatch, WorldProductPriority, WorldProductRequest,
    WorldSourceEngine, sample_cinder_vault, sample_first_pilgrim_road,
};

const SEED: u64 = 0x5eed_cafe;
const REPRESENTATIVE_CHUNKS: [(&str, ChunkCoord); 5] = [
    ("ordinary", ChunkCoord::new(2, 1, -3)),
    ("pilgrim-road", ChunkCoord::new(-90, 1, 69)),
    ("water", ChunkCoord::new(563, 0, 403)),
    ("alpine-needle", ChunkCoord::new(-7, 1, -41)),
    ("cinder-vault", ChunkCoord::new(-162, 0, 103)),
];

fn voxel_hash(chunk: &Chunk) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"voxels-procedural-v16-chunk-golden-v1\0");
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
                "procedural-v16 representative batch must succeed"
            );
            Vec::new()
        }
    };
    assert_eq!(chunks.len(), REPRESENTATIVE_CHUNKS.len());
    chunks
}

#[test]
fn procedural_v16_representative_chunks_keep_their_canonical_voxels() {
    assert_eq!(GENERATOR_VERSION, 16);
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
    let ordinary_origin = ordinary.coord().world_origin();
    assert!(ordinary.voxels().contains(&Material::Air));
    assert!(
        ordinary
            .voxels()
            .iter()
            .any(|material| material.is_collidable())
    );
    assert!(!ordinary.voxels().contains(&Material::Water));
    assert!(generator_has_no_authored_or_cave_content(
        &Generator::new(SEED),
        ordinary_origin,
    ));
    let actual = chunks.iter().map(voxel_hash).collect::<Vec<_>>();
    assert_eq!(
        actual,
        [
            "a758f4802067f2b8bdd1cf30eaefe0762168eed96bdd48cb287f120258e87ae8",
            "cf264859749d76f13c7712c5be2356606a09fd0f8dbe555083e340da446534b7",
            "491a597a9fc5c6266767278937a759c719b0bc4f99ca60fd42bc102062cb0d3a",
            "4b09c9011f476b57addc86608b3ca1a67871265a8e74aa87539f8ce1f32ba74c",
            "d6d2669314ee01812c1ccdde38b5641edd56505e5a51ed6876e49e36531b2d0d",
        ]
    );

    let identity = ProceduralWorldSource::new(SEED).source_identity_hash();
    let encoded_sizes = chunks
        .iter()
        .map(|chunk| encode_chunk(chunk, identity).len())
        .collect::<Vec<_>>();
    assert_eq!(encoded_sizes, [8_304, 12_402, 4_204, 8_302, 8_304]);
}

fn generator_has_no_authored_or_cave_content(generator: &Generator, origin: [i32; 3]) -> bool {
    let max_x = origin[0] + 32;
    let max_z = origin[2] + 32;
    if !generator
        .skyline_features_anchored_in([[origin[0], origin[2]], [max_x, max_z]])
        .is_empty()
    {
        return false;
    }
    for z in origin[2]..max_z {
        for x in origin[0]..max_x {
            if sample_first_pilgrim_road(x, z).is_some() {
                return false;
            }
            for y in origin[1]..origin[1] + 32 {
                if sample_cinder_vault(x, y, z).is_some() {
                    return false;
                }
            }
        }
    }
    true
}

#[test]
fn procedural_source_adapter_is_byte_identical_to_generator_v16() {
    let generator = Generator::new(SEED);
    for ((label, coord), adapted) in REPRESENTATIVE_CHUNKS.into_iter().zip(generated_batch()) {
        assert_eq!(
            adapted,
            generator.generate_chunk(coord),
            "adapter changed {label}"
        );
    }
}
