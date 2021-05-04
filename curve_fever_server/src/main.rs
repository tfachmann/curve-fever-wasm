use anyhow::Result;
use async_tungstenite::{tungstenite::Message, WebSocketStream};
use env_logger::Env;
use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    future::{self, join},
    sink::SinkExt,
    stream::StreamExt,
};
use log::{debug, error, info, warn};
use rand::{distributions::Alphanumeric, Rng};
use smol::{Async, Task, Timer};
use std::{
    collections::HashMap,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

use curve_fever_common::{ClientMessage, Game, GridInfo, Player, ServerMessage};

type RoomList = Arc<Mutex<HashMap<String, RoomHandle>>>;

#[derive(Clone)]
struct RoomHandle {
    play: bool,
    write: UnboundedSender<(SocketAddr, ClientMessage)>,
    room: Arc<Mutex<Room>>,
}

impl RoomHandle {
    async fn run_room(&mut self, mut read: UnboundedReceiver<(SocketAddr, ClientMessage)>) {
        while let Some((addr, msg)) = read.next().await {
            if !self.room.lock().unwrap().on_message(addr, msg) {
                break;
            }
        }
    }

    async fn tick(&mut self) {
        loop {
            Timer::after(Duration::from_millis(40)).await;
            if !self.room.lock().unwrap().tick_once() {
                break;
            }
        }
    }
}

struct Room {
    name: String,
    connections: HashMap<SocketAddr, Uuid>,
    players: HashMap<Uuid, PlayerServer>,
    game: Game,
}

impl Room {
    fn new(name: String, width: u32, height: u32, line_width: u32, rotation_delta: f64) -> Self {
        Self {
            name,
            connections: HashMap::new(),
            players: HashMap::new(),
            game: Game::new(width * 2, height * 2, line_width, rotation_delta),
        }
    }

    fn running(&self) -> bool {
        !self.connections.is_empty()
    }

    fn add_player(
        &mut self,
        addr: SocketAddr,
        player_name: String,
        ws_tx: UnboundedSender<ServerMessage>,
    ) -> Result<()> {
        // generate UUID
        let id = Uuid::new_v4();

        // create player for game
        let player = Arc::new(Mutex::new(Player::new(
            id,
            &player_name,
            self.game.width,
            self.game.height,
            self.game.line_width,
            self.game.rotation_delta,
        )));

        // insert player to players
        self.game.players.insert(id, player.clone());

        // insert player to connection map, first player is the host
        if self.connections.is_empty() {
            player.lock().unwrap().host = true;
        }
        self.connections.insert(addr, id);

        // tell other players that a player has joined
        info!(
            "[{}] Player `{}` with uuid `{}` connected sucessfully",
            self.name,
            &player_name,
            id.to_string()
        );
        ws_tx.unbounded_send(ServerMessage::JoinSuccess {
            room_name: self.name.clone(),
            grid_info: GridInfo {
                width: self.game.width,
                height: self.game.height,
                line_width: self.game.line_width,
            },
            players: {
                self.players
                    .values()
                    .map(|v| v.player.clone())
                    .map(|v| *v.lock().unwrap())
                    .collect::<Vec<Player>>()
            },
            uuid: id,
        })?;

        // create player for server
        self.players.insert(
            id,
            PlayerServer {
                name: player_name.clone(),
                ws: Some(ws_tx.clone()),
                player: player.clone(),
            },
        );

        // tell other players that a player has joined
        self.broadcast(ServerMessage::NewPlayer(*player.clone().lock().unwrap()));
        Ok(())
    }

    fn tick_once(&mut self) -> bool {
        if self.running() {
            if self.game.running() {
                self.game.tick();
                self.broadcast(ServerMessage::GameState(self.game.state()));
            }
            true
        } else {
            false
        }
    }

    fn broadcast(&self, msg: ServerMessage) {
        self.connections.values().for_each(|id| {
            if let Some(ws) = &self.players.get(id).unwrap().ws {
                if let Err(e) = ws.unbounded_send(msg.clone()) {
                    error!(
                        "[{}] Failed to send broadast to {}: {}",
                        self.name,
                        self.players.get(id).unwrap().name,
                        e
                    );
                } else {
                    //info!(
                    //"[{}] Sent broadcast to {}",
                    //self.name,
                    //self.players.get(id).unwrap().name
                    //);
                }
            } else {
                error!(
                    "[{}] Failed to send broadast to player uuid {}",
                    self.name, id
                )
            }
        });
    }

    fn on_client_disconnected(&mut self, addr: SocketAddr) {
        if let Some(id) = self.connections.remove(&addr) {
            let player = self.players.get(&id).unwrap();
            let host = { player.player.lock().unwrap().host };
            info!(
                "[{}] Removed disconnected player `{}`",
                self.name,
                player.name.clone()
            );
            self.players.remove(&id).unwrap();

            let id_host = if host {
                info!("[{}] Assinging a new host...", self.name);
                // we need a new host
                match self.players.iter_mut().next() {
                    Some((id, player)) => {
                        player.player.lock().unwrap().host = true;
                        *id
                    }
                    None => id.clone(),
                }
            } else {
                id.clone()
            };

            self.broadcast(ServerMessage::PlayerDisconnected(id, id_host))
        }
    }

    fn on_start_game(&mut self) {
        // initialize game
        self.game.initialize();

        self.broadcast(ServerMessage::GameState(self.game.state()));
        self.broadcast(ServerMessage::RoundStarted);

        //for _ in 0..100 {
        //self.game.tick();
        //self.broadcast(ServerMessage::GameState(self.game.state()));
        //}
    }

    fn on_message(&mut self, addr: SocketAddr, msg: ClientMessage) -> bool {
        info!(
            "[{}] Got message from `{}`: {:?}",
            self.name,
            self.connections
                .get(&addr)
                .map(|id| self.players.get(id).unwrap().name.clone())
                .unwrap_or_else(|| format!("unknown player at {}", addr)),
            msg
        );
        match msg {
            ClientMessage::Move(direction) => {
                if let Some(id) = self.connections.get(&addr) {
                    let player = &self.players.get(id).unwrap();
                    let uuid = { player.player.lock().unwrap().uuid };
                    if let Err(e) = self.game.on_move(&uuid, direction) {
                        error!("[{}] Error occurd during move: {}", self.name, e);
                    }
                }
            }
            ClientMessage::CreateRoom(_) | ClientMessage::JoinRoom(_, _) => {
                warn!("[{}] Invalid message", self.name);
            }
            ClientMessage::Disconnected => self.on_client_disconnected(addr),
            ClientMessage::StartGame => {
                if let Some(id) = self.connections.get(&addr) {
                    let player = &self.players.get(id).unwrap();
                    if player.player.lock().unwrap().host {
                        // valid
                        self.on_start_game();
                    } else {
                        warn!("[{}] Only the host can start a game", self.name);
                    }
                }
            }
        };
        self.running()
    }
}

struct PlayerServer {
    name: String,
    ws: Option<UnboundedSender<ServerMessage>>,
    player: Arc<Mutex<Player>>,
}

fn next_room_name(rooms: &mut HashMap<String, RoomHandle>, handle: RoomHandle) -> String {
    loop {
        let candidate: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(7)
            .map(char::from)
            .collect();
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(v) = rooms.entry(candidate.clone()) {
            v.insert(handle);
            return candidate;
        }
    }
}

async fn run_player(
    player_name: String,
    addr: SocketAddr,
    handle: RoomHandle,
    ws_stream: WebSocketStream<Async<TcpStream>>,
) {
    let (incoming, outgoing) = ws_stream.split();

    let (ws_tx, ws_rx) = unbounded();

    {
        // lock the room to add the player
        let room = &mut handle.room.lock().unwrap();
        if let Err(e) = room.add_player(addr, player_name.clone(), ws_tx) {
            error!("[{}] Failed to add player: {:?}", room.name, e);
            return;
        }
    }

    let write = handle.write.clone();
    let ra = ws_rx
        .map(|c| bincode::serialize(&c).unwrap_or_else(|_| panic!("Could not encode {:?}", c)))
        .map(Message::Binary)
        .map(Ok)
        .forward(incoming);
    let rb = outgoing
        .map(|m| match m {
            Ok(Message::Binary(t)) => bincode::deserialize::<ClientMessage>(&t).ok(),
            _ => None,
        })
        .take_while(|m| future::ready(m.is_some()))
        .map(|m| m.unwrap())
        .chain(futures::stream::once(async { ClientMessage::Disconnected }))
        .map(move |m| Ok((addr, m)))
        .forward(write);
    let (ra, rb) = join(ra, rb).await;

    if let Err(e) = ra {
        error!(
            "[{}] Got error {} from player {}'s rx queue",
            addr, e, player_name
        );
    }
    if let Err(e) = rb {
        error!(
            "[{}] Got error {} from player {}'s tx queue",
            addr, e, player_name
        );
    }
    info!("[{}] Finished session with {}", addr, player_name);
}

async fn read_stream(
    mut stream: WebSocketStream<Async<TcpStream>>,
    addr: SocketAddr,
    rooms: RoomList,
    mut close_room: UnboundedSender<String>,
) -> Result<()> {
    // do something when connected

    // read client messages
    while let Some(Ok(Message::Binary(t))) = stream.next().await {
        let msg = bincode::deserialize::<ClientMessage>(&t)?;
        info!("Received and deserialized msg");
        match msg {
            ClientMessage::CreateRoom(player_name) => {
                // create room
                let (write, read) = unbounded();
                let room = Arc::new(Mutex::new(Room::new(
                    "Testing Room".into(),
                    500, // width
                    400, // height
                    2,   // line width in px
                    2.,  // rotation delta in deg
                )));
                let handle = RoomHandle {
                    play: false,
                    write,
                    room,
                };

                let room_name = next_room_name(&mut rooms.lock().unwrap(), handle.clone());
                info!(
                    "[{}] Creating room `{}` for player {}",
                    addr, room_name, player_name
                );
                handle.room.lock().unwrap().name = room_name.clone();

                //let mut h = handle.clone();

                join(
                    handle.clone().tick(),
                    join(
                        handle.clone().run_room(read),
                        run_player(player_name, addr, handle, stream),
                    ),
                )
                .await;

                info!("[{}] All players left, closing room", room_name);
                if let Err(e) = close_room.send(room_name.clone()).await {
                    error!("[{}] Failed to close room: `{}`", room_name, e);
                }

                return Ok(());
            }
            ClientMessage::JoinRoom(player_name, room_name) => {
                info!(
                    "[{}] Player `{}` tries to join room `{}`",
                    addr, player_name, room_name
                );

                let handle = rooms.lock().unwrap().get_mut(&room_name).cloned();

                if let Some(h) = handle {
                    // room exists
                    // TODO: check for maximum amount of clients?
                    run_player(player_name, addr, h, stream).await;
                    return Ok(());
                } else {
                    // room doesn't exist
                    warn!("[{}] Room `{}` does not exist!", addr, room_name);
                    let msg =
                        ServerMessage::JoinFailed(format!("Room `{}` does not exist", room_name));
                    stream
                        .send(Message::Binary(bincode::serialize(&msg)?))
                        .await?;
                }
            }
            msg => {
                warn!("[{}] Got unexpected message {:?}", addr, msg);
                //break;
            }
        }
    }
    info!("[{}] Dropping connection", addr);
    Ok(())
}

pub fn main() {
    env_logger::from_env(Env::default().default_filter_or("curve_fever_server=INFO")).init();
    let addr = "0.0.0.0:8090";

    let rooms = Arc::new(Mutex::new(HashMap::new()));

    for _ in 0..20 {
        std::thread::spawn(|| smol::run(future::pending::<()>()));
    }

    let close_room = {
        let (tx, mut rx) = unbounded();
        let rooms = rooms.clone();
        Task::spawn(async move {
            while let Some(room) = rx.next().await {
                info!("[{}] Room closed", room);
                rooms.lock().unwrap().remove(&room);
            }
        })
        .detach();
        tx
    };

    smol::block_on(async {
        info!("Listening on: {}", addr);

        let socket_addr: SocketAddr = addr.parse().expect("Unable to parse socket address");
        let listener = Async::<TcpListener>::bind(socket_addr).expect("Could not create listener");

        while let Ok((stream, addr)) = listener.accept().await {
            info!("Got connection from {}", addr);
            let close_room = close_room.clone();
            let rooms = rooms.clone();
            Task::spawn(async move {
                match async_tungstenite::accept_async(stream).await {
                    Err(e) => {
                        error!("Could not get stream: {}", e);
                    }
                    Ok(ws_stream) => {
                        info!("Reading incoming stream...");
                        if let Err(e) = read_stream(ws_stream, addr, rooms, close_room).await {
                            error!("Failed to read stream from {}: {}", addr, e);
                        }
                    }
                };
            })
            .detach();
        }
    });
}
