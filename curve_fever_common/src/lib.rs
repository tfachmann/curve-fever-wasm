use arrayvec::ArrayString;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::TryInto, fmt, ops::{Deref, DerefMut}, sync::{Arc, Mutex}};
use uuid::Uuid;

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
    pub name: ArrayString<20>,
    pub color: ArrayString<7>,

    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    rotation_delta: f64,
    direction: Direction,

    pub x_max: u32,
    pub y_max: u32,
    pub line_width: u32,
    speed: f64,
    stop_count: f64,

    x_prev_range: (usize, usize),
    y_prev_range: (usize, usize),
}

impl Player {
    pub fn new(
        uuid: Uuid,
        name: &str,
        color: ArrayString<7>,
        x_max: u32,
        y_max: u32,
        line_width: u32,
        rotation_delta: f64,
    ) -> Self {
        Self {
            uuid,
            host: false,
            name: ArrayString::<20>::from(name).unwrap(),
            color,
            x: 0.,
            y: 0.,
            rotation: 0.,
            rotation_delta,
            direction: Direction::Unchanged,
            x_max,
            y_max,
            line_width,
            speed: 0.8,
            stop_count: 0.,
            x_prev_range: (0, 0),
            y_prev_range: (0, 0),
        }
    }

    fn initialize(&mut self) {
        let mut rng = thread_rng();
        self.x = rng.gen_range(0..self.x_max).into();
        self.y = rng.gen_range(0..self.y_max).into();
        self.rotation = rng
            .gen_range(0..(360 as f64 / self.rotation_delta as f64) as u32)
            .into();
    }

    pub fn tick(&mut self) {
        self.stop_count -= 1.;
        if self.stop_count > 0. {
            return;
        }
        self.stop_count = self.line_width as f64 - (self.line_width as f64 * self.speed);
        println!(
            "{}: ({} - {}), {}",
            self.name, self.x, self.y, self.rotation
        );
        // change rotation
        match self.direction {
            Direction::Left => self.rotation += self.rotation_delta,
            Direction::Right => self.rotation -= self.rotation_delta,
            Direction::Unchanged => (),
        }

        // change position is relative to linewidth
        let x_change = self.rotation.to_radians().cos() * (self.line_width as f64);
        let y_change = self.rotation.to_radians().sin() * (self.line_width as f64);

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

#[derive(Clone, Debug)]
pub struct Grid {
    data: Vec<Vec<Uuid>>
}

impl Grid {
    fn new(width: usize, height: usize) -> Self {
        Self {
            data: vec![vec![Uuid::default();  width]; height],
        }
    }
}

impl Deref for Grid {
    type Target = Vec<Vec<Uuid>>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for Grid {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl fmt::Display for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in self.iter() {
            for el in row.iter() {
                if *el == Uuid::default() {
                    write!(f, " ")?;
                } else {
                    write!(f, "x")?;
                }
            }
            write!(f, "\n")?;
        }
        write!(f, "\n")
    }
}

#[derive(Clone, Debug)]
pub struct Game {
    pub width: usize,  // pixel width
    pub height: usize, // pixel height
    pub line_width: u32,
    pub rotation_delta: f64,

    grid: Arc<Mutex<Grid>>, // grid with x and y pixels mapping to uuid of player

    pub players: HashMap<Uuid, Arc<Mutex<Player>>>,
    active_players: HashMap<Uuid, Arc<Mutex<Player>>>,
}

impl Game {
    pub fn new(width: usize, height: usize, line_width: u32, rotation_delta: f64) -> Self {
        let players = HashMap::new();
        let active_players = HashMap::new();
        let grid = Arc::new(Mutex::new(Grid::new(width, height)));

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

    pub fn initialize(&mut self) {
        self.active_players = self.players.clone();
        self.active_players
            .iter_mut()
            .map(|(_id, player)| player.lock().unwrap())
            .for_each(|mut player| {
                player.initialize();
            });
    }

    pub fn state(&self) -> Vec<(Uuid, (f64, f64))> {
        self.active_players
            .iter()
            .map(|(id, player)| (id, player.lock().unwrap()))
            .map(|(id, player)| (*id, (player.x, player.y)))
            .collect()
    }

    pub fn tick(&mut self) {
        // do a move for each player
        let mut remove = vec![];
        let width = self.width;
        let height = self.height;
        //let cpy = self.clone();
        {
            let mut grid = self.grid.lock().unwrap();
            self.active_players.iter_mut().for_each(|(uuid, player)| {
                // move
                player.lock().unwrap().tick();
                let linewidth_half = player.lock().unwrap().line_width as f64 / 2.0;

                // update the grid
                // TODO: be better here. More discrete, no use of floats, ...
                let pixel_range = |value: f64, max_value: usize| {
                    let lower = value - linewidth_half + 1.0;
                    let lower: usize = match lower.is_sign_negative() {
                        true => return None, // hit a wall
                        false => lower as usize,
                    };
                    let upper = (value + linewidth_half - 1.0) as usize;
                    let upper = match upper > (max_value - 1).try_into().unwrap() {
                        true => return None, // hit a wall
                        false => upper as usize,
                    };
                    Some((lower, upper))
                };

                let check_pixels = &mut || -> Option<()> {
                    let (x_prev_range, y_prev_range) = {
                        let player = player.lock().unwrap();
                        let (x_start, x_end) = pixel_range(player.x, width)?;
                        let (y_start, y_end) = pixel_range(player.y, height)?;
                        let (x_prev_start, x_prev_end) = player.x_prev_range;
                        let (y_prev_start, y_prev_end) = player.y_prev_range;
                        for x in x_start..x_end {
                            for y in y_start..y_end {
                                // don't check with your last move
                                if (x < x_prev_start || x > x_prev_end)
                                    || (y < y_prev_start || y > y_prev_end)
                                {
                                    // player is colliding with another player
                                    if grid[y][x] != Uuid::default() {
                                        println!("COLLISION WITH ANOTHER PLAYER: ({}-{})", x, y);
                                        return None;
                                    }
                                }
                                // mark each cell with your player id
                                grid[y][x] = *uuid;
                            }
                        }
                        ((x_start, x_end), ((y_start, y_end)))
                    };
                    let mut player = player.lock().unwrap();
                    player.x_prev_range = x_prev_range;
                    player.y_prev_range = y_prev_range;
                    Some(())
                };

                if let None = check_pixels() {
                    // either inside a wall, or colliding with another player
                    //println!("{}", grid);
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

    pub fn running(&self) -> bool {
        !self.active_players.is_empty()
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
    StartGame,
    Disconnected,
    Move(Direction),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ServerMessage {
    JoinFailed(String),
    JoinSuccess {
        room_name: String,
        grid_info: GridInfo,
        players: Vec<Player>,
        uuid: Uuid,
    },
    NewPlayer(Player),
    PlayerDisconnected(Uuid, Uuid),
    RoundStarted,
    GameState(Vec<(Uuid, (f64, f64))>),
}
