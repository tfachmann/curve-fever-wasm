use lazy_static;
use std::{collections::HashMap, ops::Deref, ops::DerefMut, rc::Rc, sync::Mutex};
use wasm_bindgen::convert::FromWasmAbi;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    Blob, CanvasRenderingContext2d, Document, Element, Event, EventTarget, FileReader,
    HtmlButtonElement, HtmlCanvasElement, HtmlElement, HtmlInputElement, InputEvent, KeyboardEvent,
    MessageEvent, ProgressEvent, Text, TouchEvent, WebSocket, Window,
};

use curve_fever_common::{ClientMessage, Direction, GridInfo, Player, ServerMessage};
use uuid::Uuid;

type JsResult<T> = Result<T, JsValue>;
type JsError = Result<(), JsValue>;
type JsClosure<T> = Closure<dyn FnMut(T) -> JsError>;

macro_rules! console_log {
    ($($t:tt)*) => (web_sys::console::log_1(&format!($($t)*).into()))
}

trait OptionJsValue<T> {
    fn to_js_err(self, err_msg: &str) -> Result<T, JsValue>;
}
impl<T> OptionJsValue<T> for Option<T> {
    fn to_js_err(self, err_msg: &str) -> Result<T, JsValue> {
        self.ok_or(JsValue::from_str(err_msg))
    }
}

struct Canvas {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    width: u32,
    height: u32,
}

impl Canvas {
    fn new(base: Rc<Base>, width: u32, height: u32) -> JsResult<Canvas> {
        let canvas: HtmlCanvasElement = base
            .get_element_by_id("main_canvas")?
            .dyn_into::<HtmlCanvasElement>()?;
        canvas.set_width(width);
        canvas.set_height(height);

        let context = canvas
            .get_context("2d")?
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()?;

        context.set_line_cap("round");

        Ok(Canvas {
            canvas,
            context,
            width,
            height,
        })
    }

    fn draw(&self, from: (f64, f64), to: (f64, f64), color: &str, linewidth: f64) {
        //console_log!("Drawing Canvas... (): from ({}-{}) to ({}-{})", color, from.0, from.1, to.0, to.1);
        self.context.set_line_width(linewidth);
        self.context.set_stroke_style(&color.into());
        self.context.set_fill_style(&color.into());

        self.context.begin_path();
        let from_x = from.0;
        let from_y = from.1;
        self.context.move_to(from_x, from_y);
        //self.context.stroke();

        let to_x = to.0;
        let to_y = to.1;
        self.context.line_to(to_x, to_y);
        self.context.stroke();
    }

    fn clear(&self) {
        self.context.set_fill_style(&"#263238".into());
        self.context
            .fill_rect(0., 0., self.width.into(), self.height.into());
    }
}

#[derive(Copy, Clone)]
struct MyPlayer {
    player: Player,
    x_prev: f64,
    y_prev: f64,
}

impl MyPlayer {
    fn update_pos(&mut self, x: f64, y: f64) {
        self.x_prev = self.x;
        self.y_prev = self.y;
        self.x = x;
        self.y = y;
    }
    fn init_pos(&mut self, x: f64, y: f64) {
        self.x_prev = x;
        self.x = x;
        self.y_prev = y;
        self.y = y;
    }
}

impl Deref for MyPlayer {
    type Target = Player;

    fn deref(&self) -> &Self::Target {
        &self.player
    }
}

impl DerefMut for MyPlayer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.player
    }
}

impl From<Player> for MyPlayer {
    fn from(player: Player) -> Self {
        MyPlayer {
            player,
            x_prev: player.x,
            y_prev: player.y,
        }
    }
}

trait PlayerDraw {
    fn draw(&self, canvas: &Canvas);
}

impl PlayerDraw for MyPlayer {
    fn draw(&self, canvas: &Canvas) {
        canvas.draw(
            (self.x_prev, self.y_prev),
            (self.x, self.y),
            &self.color,
            self.line_width as f64,
        );
    }
}

struct Game {
    base: Rc<Base>,
    canvas: Canvas,
    players: HashMap<Uuid, MyPlayer>,
    running: bool,
}

impl Game {
    fn new(base: Rc<Base>, x_max: u32, y_max: u32, players: Vec<MyPlayer>) -> JsResult<Game> {
        let canvas = Canvas::new(base.clone(), x_max, y_max)?;
        let players = {
            let mut map = HashMap::new();
            players.iter().for_each(|player| {
                map.insert(player.uuid, *player);
            });
            map
        };
        canvas.clear();

        Ok(Game {
            base,
            canvas,
            players,
            running: false,
        })
    }

    fn on_keydown(&mut self, event: KeyboardEvent) -> JsError {
        console_log!("Key pressed - {}", event.key().as_str());
        if self.running {
            match event.key().as_str() {
                "ArrowLeft" | "h" | "a" => self.base.send(ClientMessage::Move(Direction::Left))?,
                "ArrowRight" | "l" | "d" => {
                    self.base.send(ClientMessage::Move(Direction::Right))?
                }
                _ => (),
            }
        } else {
            match event.key().as_str() {
                " " => self.base.send(ClientMessage::StartGame)?,
                _ => (),
            }
        }
        Ok(())
    }

    fn on_keyup(&mut self, event: KeyboardEvent) -> JsError {
        if self.running {
            match event.key().as_str() {
                "ArrowLeft" | "h" | "a" => {
                    self.base.send(ClientMessage::Move(Direction::Unchanged))?
                }
                "ArrowRight" | "l" | "d" => {
                    self.base.send(ClientMessage::Move(Direction::Unchanged))?
                }
                _ => (),
            }
        }
        Ok(())
    }

    fn add_player(&mut self, player: MyPlayer) -> JsError {
        self.players.insert(player.uuid, player);
        Ok(())
    }

    fn remove_player(&mut self, uuid: Uuid, uuid_host: Uuid) -> JsError {
        (*self
            .players
            .get_mut(&uuid_host)
            .ok_or_else(|| format!("Player with uuid `{}` not found", uuid_host.to_string()))?)
        .host = true;
        self.players
            .remove(&uuid)
            .ok_or_else(|| format!("Player with uuid `{}` not found", uuid.to_string()))?;
        Ok(())
    }

    fn game_update(&mut self, game_state: Vec<(Uuid, (f64, f64))>) -> JsError {
        if self.running {
            game_state.iter().for_each(|(id, (x, y))| {
                self.players.get_mut(id).unwrap().update_pos(*x, *y);
            });
        } else {
            // initializing
            game_state.iter().for_each(|(id, (x, y))| {
                self.players.get_mut(id).unwrap().init_pos(*x, *y);
            });
        };
        self.draw()?;
        Ok(())
    }

    fn game_tick(&mut self) -> JsError {
        //self.players
        //.iter_mut()
        //.for_each(|(_id, player)| player.tick());
        self.draw()
    }

    fn draw(&mut self) -> JsError {
        self.players
            .iter()
            .for_each(|(_id, player)| player.draw(&self.canvas));
        Ok(())
    }
}

#[derive(Clone)]
struct Base {
    doc: Document,
    ws: WebSocket,
    touch: bool,
}

impl Base {
    fn send(&self, msg: ClientMessage) -> JsError {
        let encoded = bincode::serialize(&msg)
            .map_err(|e| JsValue::from_str(&format!("Could not encode: {}", e)))?;
        self.ws.send_with_u8_array(&encoded[..])
    }

    fn get_element_by_id(&self, id: &str) -> JsResult<Element> {
        Ok(self
            .doc
            .get_element_by_id(id)
            .to_js_err(&format!("Could not find id: {}", id))?)
    }
}

struct Playing {
    base: Rc<Base>,
    window: Rc<Window>,
    game: Game,

    uuid: Uuid,
    players_div: HtmlElement,
    chat_div: HtmlElement,
    handle_id: i32,
}

impl Playing {
    fn new(
        base: Rc<Base>,
        window: Rc<Window>,
        game: Game,
        room_name: String,
        uuid: Uuid,
    ) -> JsResult<Playing> {
        // show game
        base.get_element_by_id("game")?
            .set_attribute("class", "visible")?;

        base.get_element_by_id("room_name")?
            .set_inner_html(&room_name);

        let players_div = base
            .get_element_by_id("players")?
            .dyn_into::<HtmlElement>()?;
        let chat_div = base.get_element_by_id("chat")?.dyn_into::<HtmlElement>()?;

        Ok(Playing {
            base,
            window,
            game,
            uuid,
            players_div,
            chat_div,
            handle_id: 0,
        })
    }

    fn on_keydown(&mut self, event: KeyboardEvent) -> JsError {
        self.game.on_keydown(event)
    }

    fn on_keyup(&mut self, event: KeyboardEvent) -> JsError {
        self.game.on_keyup(event)
    }

    fn add_player(&mut self, player: Player) -> JsError {
        self.game.add_player(player.into())?;
        self.draw_player()?;
        Ok(())
    }

    fn remove_player(&mut self, uuid: Uuid, uuid_host: Uuid) -> JsError {
        self.game.remove_player(uuid, uuid_host)?;
        self.draw_player()?;
        Ok(())
    }

    fn game_update(&mut self, game_state: Vec<(Uuid, (f64, f64))>) -> JsError {
        self.game.game_update(game_state)?;
        Ok(())
    }

    fn round_started(&mut self) -> JsError {
        // TODO: start tick?
        // game ticks
        //let cb = Closure::wrap(Box::new(move || {
        //HANDLE
        //.lock()
        //.unwrap()
        //.game_tick()
        //.expect("Could not update game");
        //}) as Box<dyn Fn()>);

        //self.handle_id = self
        //.window
        //.set_interval_with_callback_and_timeout_and_arguments_0(
        //cb.as_ref().unchecked_ref(),
        //15,
        //)?;
        //cb.forget();

        self.game.running = true;
        Ok(())
    }

    fn draw_player(&self) -> JsError {
        self.players_div.set_inner_html("");
        for (id, player) in &self.game.players {
            let p = self.base.doc.create_element("p")?;
            p.set_class_name("player_entry");
            p.set_attribute("style", &format!("color: {}", player.color.as_str()))?;
            p.set_text_content(Some(player.name.as_str()));
            if player.host {
                let host = self.base.doc.create_element("span")?;
                host.set_class_name("host");
                host.set_text_content(Some("*"));
                p.append_child(&host)?;
            }
            if *id == self.uuid {
                let you = self.base.doc.create_element("span")?;
                you.set_class_name("you");
                you.set_text_content(Some(" (You)"));
                p.append_child(&you)?;
            }
            self.players_div.append_child(&p)?;
        }
        Ok(())
    }
}

struct MyHtmlInputElement {
    element: HtmlInputElement,
    prev_value: String,
    max_len: u32,
}

impl MyHtmlInputElement {
    fn new(element: HtmlInputElement, max_len: u32) -> Self {
        Self {
            element: element.clone(),
            prev_value: element.value(),
            max_len,
        }
    }

    fn value(&self) -> String {
        self.element.value()
    }

    fn check_name(&self, name: &str) -> bool {
        if name.len() == 0 {
            true
        } else if name.len() as u32 > self.max_len {
            false
        } else if name.contains("<") || name.contains(">") {
            false
        } else {
            true
        }
    }

    fn set_value(&mut self, val: &str) {
        if self.check_name(val) {
            // accept
            self.element.set_value(val);
            self.prev_value = self.element.value();
        } else {
            // decline
            self.element.set_value(&self.prev_value);
        }
    }
}

struct Join {
    base: Rc<Base>,
    window: Rc<Window>,

    input_name: MyHtmlInputElement,
    input_room: MyHtmlInputElement,
    join_button: HtmlButtonElement,
    err_div: HtmlElement,

    create: bool,
}

impl Drop for Join {
    fn drop(&mut self) {
        self.base
            .get_element_by_id("start")
            .unwrap()
            .set_attribute("class", "hidden")
            .unwrap();
    }
}

impl Join {
    fn new(base: Rc<Base>, window: Rc<Window>) -> JsResult<Self> {
        // input fields
        let input_name = MyHtmlInputElement::new(
            base.get_element_by_id("join_name")?
                .dyn_into::<HtmlInputElement>()?,
            20,
        );
        set_event_cb(&input_name.element, "input", move |event: InputEvent| {
            HANDLE.lock().unwrap().on_input_name(event)
        })
        .forget();

        let input_room = MyHtmlInputElement::new(
            base.get_element_by_id("join_room")?
                .dyn_into::<HtmlInputElement>()?,
            7,
        );
        set_event_cb(&input_room.element, "input", move |event: InputEvent| {
            HANDLE.lock().unwrap().on_input_room(event)
        })
        .forget();

        // error div
        let err_div = base
            .get_element_by_id("join_error")?
            .dyn_into::<HtmlElement>()?;
        err_div.set_inner_html("");

        // click for create or join button
        let join_button = base
            .get_element_by_id("create_or_join")?
            .dyn_into::<HtmlButtonElement>()?;

        let form = base.get_element_by_id("join_form")?;
        set_event_cb(&form, "submit", move |e: Event| {
            e.prevent_default();
            HANDLE.lock().unwrap().on_create_or_join()
        })
        .forget();

        Ok(Self {
            base,
            window,
            input_name,
            input_room,
            join_button,
            err_div,
            create: true,
        })
    }

    fn input_room_changed(&mut self) -> JsError {
        self.input_room.set_value(&self.input_room.value());
        if self.input_room.value().is_empty() {
            self.join_button.set_inner_html("Create new room");
            self.create = true;
        } else {
            self.join_button.set_inner_html("Join existing room");
            self.create = false;
        }
        Ok(())
    }

    fn input_name_changed(&mut self) -> JsError {
        self.input_name.set_value(&self.input_name.value());
        if self.input_name.value().chars().all(|c| c == ' ') {
            self.input_name.set_value("");
        }
        Ok(())
    }

    fn create_or_join_clicked(&self) -> JsError {
        if !self.input_name.value().is_empty() {
            self.err_div.set_inner_html("");
            let msg = match self.create {
                true => ClientMessage::CreateRoom(self.input_name.value()),
                false => ClientMessage::JoinRoom(self.input_name.value(), self.input_room.value()),
            };
            self.base.send(msg)?;
        }
        Ok(())
    }

    fn join_failed(&self, err: &str) -> JsError {
        self.err_div.set_inner_html(err);
        Ok(())
    }
}

enum State {
    Join(Join),
    Playing(Playing),
    Empty,
}

impl State {
    fn on_keydown(&mut self, event: KeyboardEvent) -> JsError {
        Ok(match self {
            State::Playing(s) => s.on_keydown(event)?,
            _ => (),
        })
    }

    fn on_keyup(&mut self, event: KeyboardEvent) -> JsError {
        Ok(match self {
            State::Playing(s) => s.on_keyup(event)?,
            _ => (),
        })
    }

    fn on_input_room(&mut self, _event: InputEvent) -> JsError {
        Ok(match self {
            State::Join(s) => s.input_room_changed()?,
            _ => (),
        })
    }

    fn on_input_name(&mut self, _event: InputEvent) -> JsError {
        Ok(match self {
            State::Join(s) => s.input_name_changed()?,
            _ => (),
        })
    }

    fn on_create_or_join(&mut self) -> JsError {
        Ok(match self {
            State::Join(s) => s.create_or_join_clicked()?,
            _ => (),
        })
    }

    fn on_join_failed(&mut self, err_text: &str) -> JsError {
        Ok(match self {
            State::Join(s) => s.join_failed(err_text)?,
            _ => (),
        })
    }

    fn on_join_success(
        &mut self,
        room_name: String,
        grid_info: GridInfo,
        players: Vec<Player>,
        uuid: Uuid,
    ) -> JsError {
        Ok(match self {
            State::Join(s) => {
                // switch state to `Playing`
                let game = Game::new(
                    s.base.clone(),
                    grid_info.width,
                    grid_info.height,
                    players
                        .iter()
                        .map(|v| (*v).into())
                        .collect::<Vec<MyPlayer>>(),
                )?;
                let s = std::mem::replace(self, State::Empty);
                match s {
                    State::Join(s) => {
                        *self = State::Playing(Playing::new(
                            s.base.clone(),
                            s.window.clone(),
                            game,
                            room_name,
                            uuid,
                        )?)
                    }
                    _ => panic!("Invalid state"),
                }
            }
            _ => (),
        })
    }

    fn on_new_player(&mut self, player: Player) -> JsError {
        Ok(match self {
            State::Playing(s) => {
                s.add_player(player)?;
            }
            _ => (),
        })
    }

    fn on_player_disconnected(&mut self, uuid: Uuid, uuid_host: Uuid) -> JsError {
        Ok(match self {
            State::Playing(s) => {
                s.remove_player(uuid, uuid_host)?;
            }
            _ => (),
        })
    }

    fn on_round_started(&mut self) -> JsError {
        Ok(match self {
            State::Playing(s) => {
                s.round_started()?;
            }
            _ => (),
        })
    }

    fn game_update(&mut self, game_state: Vec<(Uuid, (f64, f64))>) -> JsError {
        Ok(match self {
            State::Playing(s) => {
                s.game_update(game_state)?;
            }
            _ => (),
        })
    }

    fn game_tick(&mut self) -> JsError {
        Ok(match self {
            State::Playing(s) => {
                s.game.game_tick()?;
            }
            _ => (),
        })
    }
}

unsafe impl Send for State {
    /* YOLO */
}

lazy_static::lazy_static! {
    static ref HANDLE: Mutex<State> = Mutex::new(State::Empty);
}

// Boilerplate to wrap and bind a callback.
// The resulting callback must be stored for as long as it may be used.
#[must_use]
fn build_cb<F, T>(f: F) -> JsClosure<T>
where
    F: FnMut(T) -> JsError + 'static,
    T: FromWasmAbi + 'static,
{
    Closure::wrap(Box::new(f) as Box<dyn FnMut(T) -> JsError>)
}

#[must_use]
fn set_event_cb<E, F, T>(obj: &E, name: &str, f: F) -> JsClosure<T>
where
    E: JsCast + Clone + std::fmt::Debug,
    F: FnMut(T) -> JsError + 'static,
    T: FromWasmAbi + 'static,
{
    let cb = build_cb(f);
    let target = obj
        .dyn_ref::<EventTarget>()
        .expect("Could not convert into `EventTarget`");
    target
        .add_event_listener_with_callback(name, cb.as_ref().unchecked_ref())
        .expect("Could not add event listener");
    cb
}

/// Handle received message from Server
fn on_message(msg: ServerMessage) -> JsError {
    //console_log!("Received Message");
    let mut state = HANDLE.lock().unwrap();
    match msg {
        ServerMessage::GameState(game_state) => state.game_update(game_state)?,
        ServerMessage::JoinFailed(err_text) => state.on_join_failed(&err_text)?,
        ServerMessage::JoinSuccess {
            room_name,
            grid_info,
            players,
            uuid,
        } => state.on_join_success(room_name, grid_info, players, uuid)?,
        ServerMessage::NewPlayer(player) => state.on_new_player(player)?,
        ServerMessage::PlayerDisconnected(uuid, uuid_host) => {
            state.on_player_disconnected(uuid, uuid_host)?
        }
        ServerMessage::RoundStarted => state.on_round_started()?,
    };
    Ok(())
}

#[wasm_bindgen(start)]
pub fn main() -> JsError {
    console_log!("Started main!");
    let window = web_sys::window().to_js_err("no global window exists")?;

    let doc = window
        .document()
        .to_js_err("should have a document on window")?;
    let location = doc.location().to_js_err("Could not get doc location")?;
    let hostname = location.hostname()?;
    let (ws_protocol, ws_port) = if location.protocol()? == "https:" {
        ("wss", 8091)
    } else {
        ("ws", 8090)
    };
    let hostname = format!("{}://{}:{}", ws_protocol, hostname, ws_port);

    let ws = WebSocket::new(&hostname)?;

    // callback when message received
    let on_decoded_cb = Closure::wrap(Box::new(move |e: ProgressEvent| {
        let target = e.target().expect("Could not get target");
        let reader: FileReader = target.dyn_into().expect("Could not cast");
        let result = reader.result().expect("Could not get result");
        let buf = js_sys::Uint8Array::new(&result);
        let mut data = vec![0; buf.length() as usize];
        buf.copy_to(&mut data[..]);
        let msg = bincode::deserialize(&data[..])
            .map_err(|e| JsValue::from_str(&format!("Failed to deserialize: {}", e)))
            .expect("Could not decode message");
        on_message(msg).expect("Message decoding failed")
    }) as Box<dyn FnMut(ProgressEvent)>);

    // register callback
    set_event_cb(&ws, "message", move |e: MessageEvent| {
        let blob = e.data().dyn_into::<Blob>()?;
        let fr = FileReader::new()?;
        fr.add_event_listener_with_callback("load", &on_decoded_cb.as_ref().unchecked_ref())?;
        fr.read_as_array_buffer(&blob)?;
        Ok(())
    })
    .forget();

    let base = Base {
        doc,
        ws,
        touch: false,
    };

    set_event_cb(&base.doc, "keydown", move |event: KeyboardEvent| {
        HANDLE.lock().unwrap().on_keydown(event)
    })
    .forget();

    set_event_cb(&base.doc, "keyup", move |event: KeyboardEvent| {
        HANDLE.lock().unwrap().on_keyup(event)
    })
    .forget();

    *HANDLE.lock().unwrap() = State::Join(Join::new(Rc::new(base), Rc::new(window))?);
    Ok(())
}
