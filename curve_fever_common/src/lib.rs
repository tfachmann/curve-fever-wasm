use std::{
    collections::HashMap,
    convert::TryInto,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use arrayvec::ArrayString;

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum Direction {
    Left,
    Right,
    Unchanged,
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct Player {
    pub uuid: Uuid,
    pub host: bool,
    pub name: ArrayString::<20>,
    //pub color: [char; 7],
    pub color: ArrayString::<7>,

    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    rotation_delta: f64,
    direction: Direction,

    pub x_max: u32,
    pub y_max: u32,
    pub line_width: u32,
}

impl Player {
    pub fn new(uuid: Uuid, name: &str, x_max: u32, y_max: u32, line_width: u32, rotation_delta: f64) -> Self {
        Self {
            uuid,
            host: false,
            name: ArrayString::<20>::from(name).unwrap(),
            color: ArrayString::<7>::from("#E65100").unwrap(),
            x: 0.,
            y: 0.,
            rotation: 0.,
            rotation_delta,
            direction: Direction::Unchanged,
            x_max,
            y_max,
            line_width,
        }
    }

    pub fn tick(&mut self) {
        // change rotation
        match self.direction {
            Direction::Left => self.rotation += self.rotation_delta,
            Direction::Right => self.rotation -= self.rotation_delta,
            Direction::Unchanged => (),
        }

        // change position
        let x_change = self.rotation.to_radians().cos();
        let y_change = self.rotation.to_radians().sin();

        self.x += x_change;
        if self.x < 0. {
            self.x = 0.;
        }
        if self.x > self.x_max as f64 {
            self.x = self.x_max as f64;
        }

        self.y += y_change;
        if self.y < 0. {
            self.y = 0.;
        }
        if self.y > self.y_max as f64 {
            self.y = self.y_max as f64;
        }
    }

    fn change_direction(&mut self, direction: Direction) {
        self.direction = direction;
    }
}

pub struct Game {
    pub width: u32,  // pixel width
    pub height: u32, // pixel height
    pub line_width: u32,
    pub rotation_delta: f64,

    grid: Arc<Mutex<Vec<Vec<Uuid>>>>, // grid with x and y pixels mapping to uuid of player

    pub players: HashMap<Uuid, Arc<Mutex<Player>>>,
    active_players: HashMap<Uuid, Arc<Mutex<Player>>>,
}

impl Game {
    pub fn new(width: u32, height: u32, line_width: u32, rotation_delta: f64) -> Self {
        let players = HashMap::new();
        let active_players = HashMap::new();
        let grid = Arc::new(Mutex::new(Vec::new()));

        Self {
            width,
            height,
            line_width,
            rotation_delta,
            grid,
            players,
            active_players,
        }
    }

    pub fn tick(&mut self) {
        // do a move for each player
        let mut remove = vec![];
        let width = self.width;
        let height = self.height;
        {
            let mut grid = self.grid.lock().unwrap();
            self.active_players.iter_mut().for_each(|(uuid, player)| {
                // move
                player.lock().unwrap().tick();
                let linewidth_half = player.lock().unwrap().line_width as f64 / 0.5;

                // update the grid
                let pixel_range = |value: f64, max_value: u32| {
                    let lower = value - linewidth_half - 1.0;
                    let lower: usize = match lower.is_sign_negative() {
                        true => return None, // hit a wall
                        false => lower as usize,
                    };
                    let upper = (value + linewidth_half - 1.0) as usize;
                    let upper = match upper > (max_value - 1).try_into().unwrap() {
                        true => return None, // hit a wall
                        false => upper as usize,
                    };
                    Some(lower..upper)
                };

                let check_pixels = &mut || -> Option<()> {
                    let player = player.lock().unwrap();
                    for x in pixel_range(player.x, width)? {
                        for y in pixel_range(player.y, height)? {
                            // player is colliding with another player
                            if grid[x][y] != Uuid::default() {
                                return None;
                            }
                            grid[x][y] = *uuid;
                        }
                    }
                    Some(())
                };

                if let None = check_pixels() {
                    // either inside a wall, or colliding with another player
                    remove.push(uuid.clone());
                }
            });
        }

        // remove player from game
        remove.iter().for_each(|uuid_remove| {
            self.active_players
                .remove(uuid_remove)
                .expect("Player to be removed was not found");
        });

        // TODO: send back to client?
    }

    pub fn on_move(&mut self, id: &Uuid, direction: Direction) -> Result<(), String> {
        self.active_players
            .get_mut(id)
            .ok_or_else(|| format!("There is no player with uuid: {}", id))?
            .lock()
            .unwrap()
            .change_direction(direction);
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GridInfo {
    pub width: u32,
    pub height: u32,
    pub line_width: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ClientMessage {
    CreateRoom(String),
    JoinRoom(String, String),
    Move(Direction),
    StartGame,
    Disconnected,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ServerMessage {
    JoinFailed(String),
    JoinSuccess {
        room_name: String,
        grid_info: GridInfo,
        players: Vec<Player>,
    },
    NewPlayer(Player),
    PlayerDisconnected(Uuid, Uuid),
    GameState(Vec<(Uuid, Direction)>),
}
