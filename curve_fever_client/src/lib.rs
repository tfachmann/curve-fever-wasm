use lazy_static;
use std::{ops::Deref, ops::DerefMut, rc::Rc, sync::Mutex};
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
    width_scaled: u32,
    height_scaled: u32,
}

impl Canvas {
    fn new(base: Rc<Base>, width: u32, height: u32) -> JsResult<Canvas> {
        let canvas: HtmlCanvasElement = base
            .get_element_by_id("main_canvas")?
            .dyn_into::<HtmlCanvasElement>()?;

        canvas.set_attribute("class", "visible")?;
        canvas.set_width(width * 2);
        canvas.set_height(height * 2);

        let context = canvas
            .get_context("2d")?
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()?;

        let width_scaled = canvas.width() / width;
        let height_scaled = canvas.height() / height;

        context.set_line_width(width_scaled as f64);
        context.set_line_cap("round");

        Ok(Canvas {
            canvas,
            context,
            width,
            height,
            width_scaled,
            height_scaled,
        })
    }

    fn draw(&self, from: (f64, f64), to: (f64, f64), color: &str) {
        //console_log!("Drawing Canvas... {}: {} {}", color, x, y);
        self.context.set_stroke_style(&color.into());
        self.context.set_fill_style(&color.into());

        self.context.begin_path();
        let from_x = from.0 * self.width_scaled as f64;
        let from_y = from.1 * self.height_scaled as f64;
        self.context.move_to(from_x, from_y);
        //self.context.stroke();

        let to_x = to.0 * self.width_scaled as f64;
        let to_y = to.1 * self.height_scaled as f64;
        self.context.line_to(to_x, to_y);
        self.context.stroke();
    }

    fn clear(&self) {
        self.context.set_fill_style(&"#263238".into());
        self.context.fill_rect(
            0.,
            0.,
            (self.width * self.width_scaled).into(),
            (self.height * self.height_scaled).into(),
        );
    }
}

struct MyPlayer {
    player: Player,
    x_prev: f64,
    y_prev: f64,
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
    fn tick(&mut self);
    fn draw(&self, canvas: &Canvas);
}

impl PlayerDraw for MyPlayer {
    fn tick(&mut self) {
        //console_log!("{} - {}", self.x, self.y);
        let x_change = self.rotation.to_radians().cos();
        let y_change = self.rotation.to_radians().sin();

        self.x_prev = self.x;
        self.y_prev = self.y;

        self.x += x_change;
        if self.x < 0. {
            self.x = 0.;
            return;
        }
        if self.x > self.x_max as f64 {
            self.x = self.x_max as f64;
            return;
        }

        self.y += y_change;
        if self.y < 0. {
            self.y = 0.;
            self.x = self.x_prev;
            return;
        }
        if self.y > self.y_max as f64 {
            self.y = self.y_max as f64;
            self.x = self.x_prev;
            return;
        }
    }

    fn draw(&self, canvas: &Canvas) {
        canvas.draw((self.x_prev, self.y_prev), (self.x, self.y), &self.color);
    }
}

struct Game {
    base: Rc<Base>,
    canvas: Canvas,
    players: Vec<MyPlayer>,
}

impl Game {
    fn new(base: Rc<Base>, x_max: u32, y_max: u32, players: Vec<MyPlayer>) -> JsResult<Game> {
        let canvas = Canvas::new(base.clone(), x_max, y_max)?;
        canvas.clear();

        Ok(Game {
            base,
            canvas,
            players,
        })
    }

    fn on_keydown(&mut self, event: KeyboardEvent) -> JsError {
        console_log!("Key pressed - {}", event.key().as_str());
        match event.key().as_str() {
            "ArrowLeft" | "h" | "a" => self.base.send(ClientMessage::Move(Direction::Left))?,
            "ArrowRight" | "l" | "d" => self.base.send(ClientMessage::Move(Direction::Right))?,
            _ => (),
        }
        Ok(())
    }

    fn on_keyup(&mut self, event: KeyboardEvent) -> JsError {
        console_log!("Key up - {}", event.key().as_str());
        match event.key().as_str() {
            "ArrowLeft" | "h" | "a" => self.base.send(ClientMessage::Move(Direction::Unchanged))?,
            "ArrowRight" | "l" | "d" => {
                self.base.send(ClientMessage::Move(Direction::Unchanged))?
            }
            _ => (),
        }
        Ok(())
    }

    fn game_tick(&mut self) -> JsError {
        self.players.iter_mut().for_each(|player| player.tick());
        self.draw()
    }

    fn draw(&mut self) -> JsError {
        self.players
            .iter()
            .for_each(|player| player.draw(&self.canvas));
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

    handle_id: i32,
}

impl Playing {
    fn new(base: Rc<Base>, window: Rc<Window>, game: Game) -> JsResult<Playing> {
        // show canvas

        // game ticks
        let cb = Closure::wrap(Box::new(move || {
            HANDLE
                .lock()
                .unwrap()
                .game_tick()
                .expect("Could not update game");
        }) as Box<dyn Fn()>);

        let handle_id = window.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            20,
        )?;
        cb.forget();

        Ok(Playing {
            base,
            window,
            game,
            handle_id,
        })
    }

    fn on_keydown(&mut self, event: KeyboardEvent) -> JsError {
        self.game.on_keydown(event)
    }

    fn on_keyup(&mut self, event: KeyboardEvent) -> JsError {
        self.game.on_keyup(event)
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
        if name.len() as u32 > self.max_len {
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
        self.input_room.set_value(&self.input_room.value());
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

    fn join_success(
        &mut self,
        room_name: String,
        host: Uuid,
        grid_info: GridInfo,
        players: Vec<Player>,
    ) -> JsError {
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
        host: Uuid,
        grid_info: GridInfo,
        players: Vec<Player>,
    ) -> JsError {
        Ok(match self {
            //State::Join(s) => s.join_success(room_name, host, grid_info, players)?,
            State::Join(s) => {
                // switch state to `Playing`
                let game = Game::new(
                    s.base.clone(),
                    (grid_info.width as f64 / 2.0) as u32,
                    (grid_info.height as f64 / 2.0) as u32,
                    players
                        .iter()
                        .map(|v| (*v).into())
                        .collect::<Vec<MyPlayer>>(),
                )?;
                let s = std::mem::replace(self, State::Empty);
                match s {
                    State::Join(s) => {
                        *self =
                            State::Playing(Playing::new(s.base.clone(), s.window.clone(), game)?)
                    }
                    _ => panic!("Invalid state"),
                }
            }
            _ => (),
        })
    }

    fn game_update(&mut self, game_state: Vec<(Uuid, Direction)>) -> JsError {
        Ok(match self {
            State::Playing(s) => {
                console_log!("game_update()");
            }
            _ => (),
        })
    }

    fn game_tick(&mut self) -> JsError {
        Ok(match self {
            State::Playing(s) => {
                //console_log!("game_tick()");
                s.game.game_tick()?;
                //match s.board.grid.do_move() {
                //Ok(_) => s.board.draw()?,
                //Err(_) => {
                //s.stop_game()?;
                //// Transition to EndGame
                //let s = std::mem::replace(self, State::Empty);
                //match s {
                //State::Playing(s) => *self = State::EndGame(s.on_start_game()?),
                //_ => panic!("Invalid state"),
                //}
                //}
                //}
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
    console_log!("Received Message");
    let mut state = HANDLE.lock().unwrap();
    match msg {
        ServerMessage::GameState(game_state) => state.game_update(game_state)?,
        ServerMessage::JoinFailed(err_text) => state.on_join_failed(&err_text)?,
        ServerMessage::JoinSuccess {
            room_name,
            host,
            grid_info,
            players,
        } => state.on_join_success(room_name, host, grid_info, players)?,
        ServerMessage::NewPlayer(_, _) => {}
        ServerMessage::PlayerDisconnected(_) => {}
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
