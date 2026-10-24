use std::collections::{BTreeMap, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
pub enum Direction {
    #[serde(rename = "west")]
    West,
    #[serde(rename = "north")]
    North,
    #[serde(rename = "east")]
    East,
    #[serde(rename = "south")]
    South,
}

impl Direction {
    const ORDER: [Self; 4] = [Self::West, Self::North, Self::East, Self::South];

    fn is_opposite(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::West, Self::East)
                | (Self::East, Self::West)
                | (Self::North, Self::South)
                | (Self::South, Self::North)
        )
    }
}

impl std::fmt::Display for Direction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::West => "West",
            Self::North => "North",
            Self::East => "East",
            Self::South => "South",
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortalExits {
    #[serde(default)]
    pub west: u64,
    #[serde(default)]
    pub north: u64,
    #[serde(default)]
    pub east: u64,
    #[serde(default)]
    pub south: u64,
}

impl PortalExits {
    fn destination(&self, direction: Direction) -> u64 {
        match direction {
            Direction::West => self.west,
            Direction::North => self.north,
            Direction::East => self.east,
            Direction::South => self.south,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (Direction, u64)> + '_ {
        Direction::ORDER
            .into_iter()
            .map(|direction| (direction, self.destination(direction)))
            .filter(|(_, destination)| *destination != 0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Route {
    pub zones: Vec<u64>,
    pub directions: Vec<Direction>,
}

pub fn validate_graph(graph: &BTreeMap<u64, PortalExits>) -> Result<(), String> {
    for (source, exits) in graph {
        for (direction, destination) in exits.iter() {
            if !graph.contains_key(&destination) {
                return Err(format!(
                    "zone {source} points {direction} to missing zone {destination}"
                ));
            }
        }
    }
    Ok(())
}

pub fn shortest_path(
    graph: &BTreeMap<u64, PortalExits>,
    start: u64,
    destination: u64,
) -> Option<Route> {
    if !graph.contains_key(&start) || !graph.contains_key(&destination) {
        return None;
    }

    let mut queue = VecDeque::from([start]);
    let mut distances = HashMap::from([(start, 0_usize)]);
    while let Some(zone) = queue.pop_front() {
        let Some(exits) = graph.get(&zone) else {
            continue;
        };
        for (_, next) in exits.iter() {
            if graph.contains_key(&next) && !distances.contains_key(&next) {
                distances.insert(next, distances[&zone] + 1);
                queue.push_back(next);
            }
        }
    }

    let shortest_length = *distances.get(&destination)?;
    let initial_state = (start, None);
    let mut scores = HashMap::from([(initial_state, 0_usize)]);
    let mut predecessors = HashMap::new();

    // Keep the best reversal count for each zone and incoming direction, but
    // only follow edges that stay on a shortest route to the destination.
    for depth in 0..shortest_length {
        for &zone in graph.keys() {
            let Some(&zone_distance) = distances.get(&zone) else {
                continue;
            };
            if zone_distance != depth {
                continue;
            }
            for previous_direction in [None]
                .into_iter()
                .chain(Direction::ORDER.into_iter().map(Some))
            {
                let state = (zone, previous_direction);
                let Some(&score) = scores.get(&state) else {
                    continue;
                };
                for (direction, next) in graph[&zone].iter() {
                    if distances.get(&next) != Some(&(depth + 1)) {
                        continue;
                    }
                    let next_state = (next, Some(direction));
                    let next_score = score
                        + usize::from(
                            previous_direction
                                .is_some_and(|previous| previous.is_opposite(direction)),
                        );
                    if scores
                        .get(&next_state)
                        .is_none_or(|&best| next_score > best)
                    {
                        scores.insert(next_state, next_score);
                        predecessors.insert(next_state, state);
                    }
                }
            }
        }
    }

    let mut best_state = None;
    let mut best_score = 0;
    for direction in Direction::ORDER {
        let state = (destination, Some(direction));
        if let Some(&score) = scores.get(&state) {
            if best_state.is_none() || score > best_score {
                best_state = Some(state);
                best_score = score;
            }
        }
    }
    let mut current = best_state.unwrap_or(initial_state);
    let mut zones = vec![destination];
    let mut directions = Vec::new();
    while current != initial_state {
        directions.push(current.1?);
        current = *predecessors.get(&current)?;
        zones.push(current.0);
    }
    zones.reverse();
    directions.reverse();
    Some(Route { zones, directions })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_from_json(json: &str) -> BTreeMap<u64, PortalExits> {
        serde_json::from_str(json).expect("test graph should be valid JSON")
    }

    #[test]
    fn finds_shortest_directed_route_and_ignores_zero_exits() {
        let graph = graph_from_json(
            r#"{
                "11": {"west": 12, "east": 21, "north": 0, "south": 0},
                "12": {"north": 22},
                "21": {"east": 22},
                "22": {"south": 33},
                "33": {"north": 0}
            }"#,
        );

        let route = shortest_path(&graph, 11, 33).expect("a route exists");
        assert_eq!(route.zones, [11, 12, 22, 33]);
        assert_eq!(
            route.directions,
            [Direction::West, Direction::North, Direction::South]
        );
    }

    #[test]
    fn portals_are_directed_and_unreachable_zones_have_no_route() {
        let graph = graph_from_json(
            r#"{
                "1": {"east": 2},
                "2": {"west": 0}
            }"#,
        );

        assert!(shortest_path(&graph, 2, 1).is_none());
        assert_eq!(
            shortest_path(&graph, 1, 2).unwrap().directions,
            [Direction::East]
        );
    }

    #[test]
    fn prefers_the_most_doubling_back_among_shortest_routes() {
        let graph = graph_from_json(
            r#"{
                "1": {"north": 2, "east": 3},
                "2": {"south": 4},
                "3": {"east": 6},
                "4": {"east": 5},
                "6": {"east": 5},
                "5": {}
            }"#,
        );

        let route = shortest_path(&graph, 1, 5).expect("a route exists");
        assert_eq!(route.zones, [1, 2, 4, 5]);
        assert_eq!(
            route.directions,
            [Direction::North, Direction::South, Direction::East]
        );
    }

    #[test]
    fn never_prefers_a_longer_route_for_extra_doubling_back() {
        let graph = graph_from_json(
            r#"{
                "1": {"north": 2, "east": 3},
                "2": {"south": 4},
                "3": {"north": 6},
                "4": {"east": 5},
                "6": {"south": 7},
                "7": {"east": 5},
                "5": {}
            }"#,
        );

        let route = shortest_path(&graph, 1, 5).expect("a route exists");
        assert_eq!(
            route.directions,
            [Direction::North, Direction::South, Direction::East]
        );
    }

    #[test]
    fn handles_start_equal_to_destination() {
        let graph = graph_from_json(r#"{"7": {}}"#);
        assert_eq!(
            shortest_path(&graph, 7, 7),
            Some(Route {
                zones: vec![7],
                directions: vec![]
            })
        );
    }

    #[test]
    fn validates_all_nonzero_destinations() {
        let graph = graph_from_json(r#"{"1": {"west": 99}}"#);
        assert!(
            validate_graph(&graph)
                .unwrap_err()
                .contains("missing zone 99")
        );
    }
}
