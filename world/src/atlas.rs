use crate::first_pilgrim_road_length_voxels;

/// Append-only semantic atlas schema. Names and stable IDs do not alter canonical terrain by
/// themselves, but consumers may persist discoveries and must be able to interpret older IDs.
pub const ATLAS_VERSION: u32 = 1;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DestinationId {
    #[default]
    Trailhead = 0,
    HoodooRest = 1,
    GreenwardCrossing = 2,
    Moorwatch = 3,
    CinderSteps = 4,
    NeedleGate = 5,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RouteChapterId {
    #[default]
    OldPilgrimRise = 0,
    GreenwardTraverse = 1,
    WindcutWay = 2,
    CinderReach = 3,
    NeedleAscent = 4,
}

impl RouteChapterId {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OldPilgrimRise => "OLD PILGRIM RISE",
            Self::GreenwardTraverse => "GREENWARD TRAVERSE",
            Self::WindcutWay => "WINDCUT WAY",
            Self::CinderReach => "CINDER REACH",
            Self::NeedleAscent => "NEEDLE ASCENT",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Destination {
    pub id: DestinationId,
    pub name: &'static str,
    pub route_station_voxels: i32,
    pub position: [i32; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteChapter {
    pub id: RouteChapterId,
    pub start_station_voxels: i32,
    pub end_station_voxels: i32,
    pub destination: DestinationId,
}

pub const PILGRIM_DESTINATIONS: [Destination; 6] = [
    Destination {
        id: DestinationId::Trailhead,
        name: "Trailhead",
        route_station_voxels: 0,
        position: [0, 48],
    },
    Destination {
        id: DestinationId::HoodooRest,
        name: "Hoodoo Rest",
        route_station_voxels: 1_647,
        position: [-1_180, 1_194],
    },
    Destination {
        id: DestinationId::GreenwardCrossing,
        name: "Greenward Crossing",
        route_station_voxels: 3_285,
        position: [-2_492, 1_994],
    },
    Destination {
        id: DestinationId::Moorwatch,
        name: "Moorwatch",
        route_station_voxels: 4_502,
        position: [-3_516, 2_410],
    },
    Destination {
        id: DestinationId::CinderSteps,
        name: "Cinder Steps",
        route_station_voxels: 6_222,
        position: [-4_988, 3_050],
    },
    Destination {
        id: DestinationId::NeedleGate,
        name: "Needle Gate",
        route_station_voxels: 7_530,
        position: [-5_200, 4_200],
    },
];

pub const PILGRIM_CHAPTERS: [RouteChapter; 5] = [
    RouteChapter {
        id: RouteChapterId::OldPilgrimRise,
        start_station_voxels: 0,
        end_station_voxels: 1_647,
        destination: DestinationId::HoodooRest,
    },
    RouteChapter {
        id: RouteChapterId::GreenwardTraverse,
        start_station_voxels: 1_647,
        end_station_voxels: 3_285,
        destination: DestinationId::GreenwardCrossing,
    },
    RouteChapter {
        id: RouteChapterId::WindcutWay,
        start_station_voxels: 3_285,
        end_station_voxels: 4_502,
        destination: DestinationId::Moorwatch,
    },
    RouteChapter {
        id: RouteChapterId::CinderReach,
        start_station_voxels: 4_502,
        end_station_voxels: 6_222,
        destination: DestinationId::CinderSteps,
    },
    RouteChapter {
        id: RouteChapterId::NeedleAscent,
        start_station_voxels: 6_222,
        end_station_voxels: 7_530,
        destination: DestinationId::NeedleGate,
    },
];

pub fn pilgrim_chapter_at_distance(distance_voxels: f32) -> RouteChapter {
    let distance = distance_voxels.clamp(0.0, first_pilgrim_road_length_voxels());
    PILGRIM_CHAPTERS
        .iter()
        .copied()
        .find(|chapter| distance < chapter.end_station_voxels as f32)
        .unwrap_or(PILGRIM_CHAPTERS[PILGRIM_CHAPTERS.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_pilgrim_road_point_at_distance;
    use std::collections::BTreeSet;

    #[test]
    fn atlas_ids_and_route_stations_are_stable_and_self_consistent() {
        assert_eq!(ATLAS_VERSION, 1);
        assert_eq!(PILGRIM_DESTINATIONS.len(), 6);
        assert_eq!(PILGRIM_CHAPTERS.len(), 5);
        let mut ids = BTreeSet::new();
        for destination in PILGRIM_DESTINATIONS {
            assert!(ids.insert(destination.id as u8));
            let (point, _) =
                first_pilgrim_road_point_at_distance(destination.route_station_voxels as f32)
                    .expect("destination station must resolve");
            assert!((point[0].round() as i32 - destination.position[0]).abs() <= 1);
            assert!((point[1].round() as i32 - destination.position[1]).abs() <= 1);
        }
        assert_eq!(PILGRIM_DESTINATIONS[0].id as u8, 0);
        assert_eq!(PILGRIM_DESTINATIONS[5].id as u8, 5);
        assert_eq!(PILGRIM_CHAPTERS[0].id as u8, 0);
        assert_eq!(PILGRIM_CHAPTERS[4].id as u8, 4);
    }

    #[test]
    fn chapter_lookup_is_ordered_and_covers_the_whole_route() {
        let mut previous = RouteChapterId::OldPilgrimRise;
        for distance in 0..=first_pilgrim_road_length_voxels().ceil() as i32 {
            let chapter = pilgrim_chapter_at_distance(distance as f32);
            assert!(chapter.id >= previous);
            assert!(distance as f32 >= chapter.start_station_voxels as f32 || distance == 0);
            previous = chapter.id;
        }
        assert_eq!(previous, RouteChapterId::NeedleAscent);
    }
}
