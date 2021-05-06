use arrayvec::ArrayString;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    convert::TryInto,
    fmt,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum Direction {
    Left,
    Right,
    Unchanged,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlayerState {
    pub id: Uuid,
    pub x: f64,
    pub y: f64,
    pub invisible: bool,
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

    pub invisible: bool,
    invisible_max: usize,
    invisible_count: usize,
    invisible_length: usize,

    pub points: usize,

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
            invisible: false,
            invisible_max: 100,
            invisible_count: 0,
            invisible_length: 3,
            points: 0,
            x_prev_range: (0, 0),
            y_prev_range: (0, 0),
        }
    }

    fn initialize(&mut self) {
        let mut rng = thread_rng();
        self.direction = Direction::Unchanged;
        self.invisible_count = self.invisible_max;
        let x_limits = (self.x_max as f64 * 0.15) as u32;
        let y_limits = (self.y_max as f64 * 0.15) as u32;
        self.x = rng.gen_range(0 + x_limits..self.x_max - x_limits).into();
        self.y = rng.gen_range(0 + y_limits..self.y_max - y_limits).into();
        self.rotation = self.rotation_delta
            * rng.gen_range(0..(360 as f64 / self.rotation_delta as f64) as u32) as f64;
    }

    pub fn tick(&mut self) {
        // don't move if in stop_count (handles speed by not updating)
        self.stop_count -= 1.;
        if self.stop_count > 0. {
            return;
        }
        self.stop_count = self.line_width as f64 - (self.line_width as f64 * self.speed);

        // handle invisibility
        self.invisible_count -= 1;
        if self.invisible_count == 0 {
            self.invisible = true;
            self.invisible_count = self.invisible_max;
        }

        if self.invisible && self.invisible_count < self.invisible_max - self.invisible_length {
            self.invisible = false;
        }

        // change rotation
        match self.direction {
            Direction::Left => self.rotation += self.rotation_delta,
            Direction::Right => self.rotation -= self.rotation_delta,
            Direction::Unchanged => (),
        }

        // change position is relative to linewidth
        let x_change = self.rotation.to_radians().sin() * (self.line_width as f64);
        let y_change = self.rotation.to_radians().cos() * (self.line_width as f64);

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
    data: Vec<Vec<Uuid>>,
}

impl Grid {
    fn new(width: usize, height: usize) -> Self {
        Self {
            data: vec![vec![Uuid::default(); width]; height],
        }
    }

    fn clear(&mut self) {
        self.data
            .iter_mut()
            .for_each(|row| row.iter_mut().for_each(|el| *el = Uuid::default()));
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
    single_player: bool,

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
            single_player: false,
        }
    }

    pub fn initialize(&mut self) {
        if self.players.len() == 1 {
            self.single_player = true;
        } else {
            self.single_player = false;
        }
        self.grid.lock().unwrap().clear();
        self.active_players = self.players.clone();
        self.active_players
            .iter_mut()
            .map(|(_id, player)| player.lock().unwrap())
            .for_each(|mut player| {
                player.initialize();
            });
    }

    pub fn state(&self) -> Vec<PlayerState> {
        self.active_players
            .iter()
            .map(|(id, player)| (id, player.lock().unwrap()))
            .map(|(id, player)| PlayerState {
                id: *id,
                x: player.x,
                y: player.y,
                invisible: player.invisible,
            })
            .collect()
    }

    pub fn state_ended(&self) -> Vec<(Uuid, usize)> {
        self.players
            .iter()
            .map(|(id, player)| (id, player.lock().unwrap()))
            .map(|(id, player)| (*id, player.points))
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

                if !player.lock().unwrap().invisible {
                    if let None = check_pixels() {
                        // either inside a wall, or colliding with another player
                        //println!("{}", grid);
                        remove.push(uuid.clone());
                    }
                }
            });
        }

        // remove player from game
        remove.iter().for_each(|uuid_remove| {
            if !self.single_player {
                // calculate points if not in single player
                self.calculate_points(uuid_remove);
            }
            self.active_players
                .remove(uuid_remove)
                .expect("Player to be removed was not found");
        });

        if !self.single_player {
            if self.active_players.len() == 1 {
                // we have a winner
                println!("Calculate points of winner");
                let uuid = *self.active_players.keys().next().unwrap();
                self.calculate_points(&uuid);
            }
        }
    }

    pub fn remove_player(&mut self, uuid: &Uuid) {
        self.active_players.remove(uuid);
        self.players.remove(uuid);
    }

    fn calculate_points(&mut self, uuid: &Uuid) {
        let len_total = self.players.len();
        let mut player = self.players.get_mut(uuid).unwrap().lock().unwrap();
        player.points += 2_usize.pow((len_total - self.active_players.len()).try_into().unwrap());
    }

    pub fn running(&self) -> bool {
        if self.single_player {
            !self.active_players.is_empty()
        } else {
            self.active_players.len() > 1
        }
    }

    pub fn get_winner(&self) -> Option<Uuid> {
        if !self.running() {
            if self.single_player {
                Some(*self.players.iter().next().unwrap().0)
            } else {
                Some(*self.active_players.iter().next().unwrap().0)
            }
        } else {
            None
        }
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
    RoundEnded((Uuid, Vec<(Uuid, usize)>)),
    GameState(Vec<PlayerState>),
}
