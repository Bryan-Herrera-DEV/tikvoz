//! Interfaz (egui, modo inmediato). Solo se redibuja cuando llega un evento o el
//! usuario interactúa, así que en reposo el consumo de CPU es prácticamente cero.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::config::Settings;
use crate::filter::{self, RepeatGuard};
use crate::speech::{EngineInfo, Speaker};
use crate::tiktok::{LiveClient, LiveEvent, Status};

const FEED_CAP: usize = 300;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Comment,
    Gift,
    Follow,
    Share,
}

impl Kind {
    fn icon(self) -> &'static str {
        match self {
            Kind::Comment => "💬",
            Kind::Gift => "🎁",
            Kind::Follow => "➕",
            Kind::Share => "🔗",
        }
    }
}

#[derive(Clone, PartialEq)]
enum Outcome {
    Spoken,
    Skipped(&'static str),
    History,
}

struct FeedItem {
    kind: Kind,
    nickname: String,
    handle: String,
    text: String,
    /// Lo que se leería si el usuario pulsa 🔊.
    speech: String,
    outcome: Outcome,
}

pub struct App {
    settings: Settings,
    applied: Settings,
    live: LiveClient,
    events: Receiver<LiveEvent>,
    speaker: Option<Speaker>,
    engine: Option<EngineInfo>,
    engine_error: Option<String>,
    status: Status,
    room_id: Option<i64>,
    feed: VecDeque<FeedItem>,
    repeats: RepeatGuard,
    paused: bool,
    test_text: String,
    show_api_key: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();

        let ctx = cc.egui_ctx.clone();
        let (tx, events) = mpsc::channel();
        let repaint = ctx.clone();
        let live = LiveClient::spawn(Arc::new(move |e| {
            let _ = tx.send(e);
            repaint.request_repaint();
        }))
        .expect("no se pudo iniciar el hilo de red");

        let repaint = ctx.clone();
        let (speaker, engine, engine_error) =
            match Speaker::spawn(settings.max_queue, move || repaint.request_repaint()) {
                Ok((s, info)) => (Some(s), Some(info), None),
                Err(e) => (None, None, Some(e.to_string())),
            };

        let mut app = Self {
            repeats: RepeatGuard::new(Duration::from_secs(settings.repeat_window_secs)),
            applied: settings.clone(),
            settings,
            live,
            events,
            speaker,
            engine,
            engine_error,
            status: Status::Idle,
            room_id: None,
            feed: VecDeque::with_capacity(FEED_CAP),
            paused: false,
            test_text: "Hola, así sonarán los comentarios de tu live.".into(),
            show_api_key: false,
        };
        app.pick_default_voice();
        app.push_voice_settings(true);

        // `tikvoz.exe usuario` conecta directo (útil para accesos directos).
        let cli_user = std::env::args().nth(1).filter(|u| !u.trim().is_empty());
        if let Some(user) = &cli_user {
            app.settings.username = user.clone();
        }
        if (cli_user.is_some() || app.settings.auto_connect) && !app.settings.username.trim().is_empty() {
            app.connect();
        }
        app
    }

    /// Si no hay voz guardada, prefiere una en español.
    fn pick_default_voice(&mut self) {
        let Some(engine) = &self.engine else { return };
        let saved_exists = self
            .settings
            .voice_id
            .as_ref()
            .is_some_and(|id| engine.voices.iter().any(|v| &v.id == id));
        if !saved_exists {
            self.settings.voice_id = engine
                .voices
                .iter()
                .find(|v| v.language.starts_with("es"))
                .or_else(|| engine.voices.first())
                .map(|v| v.id.clone())
                .or_else(|| engine.current_voice.clone());
        }
    }

    /// Envía al motor solo lo que cambió (o todo si `force`).
    fn push_voice_settings(&mut self, force: bool) {
        let Some(sp) = &self.speaker else { return };
        let (s, a) = (&self.settings, &self.applied);
        if force || s.voice_id != a.voice_id {
            if let Some(id) = &s.voice_id {
                sp.set_voice(id.clone());
            }
        }
        if force || s.rate != a.rate {
            sp.set_rate(s.rate);
        }
        if force || s.volume != a.volume {
            sp.set_volume(s.volume);
        }
        if force || s.max_queue != a.max_queue {
            sp.set_max_queue(s.max_queue);
        }
        if force || s.repeat_window_secs != a.repeat_window_secs {
            self.repeats
                .set_window(Duration::from_secs(s.repeat_window_secs));
        }
        self.applied = self.settings.clone();
    }

    fn connect(&mut self) {
        let user = self.settings.username.trim().trim_start_matches('@').to_string();
        if user.is_empty() {
            return;
        }
        self.settings.username = user.clone();
        self.live.connect(&user, self.settings.api_key());
    }

    fn is_active(&self) -> bool {
        !matches!(self.status, Status::Idle | Status::Error(_))
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                LiveEvent::Status(s) => self.status = s,
                LiveEvent::Connected { room_id } => self.room_id = Some(room_id),
                LiveEvent::Comment { nickname, handle, text, history } => {
                    self.on_comment(nickname, handle, text, history)
                }
                LiveEvent::Gift { nickname, handle, gift, count, diamonds, history } => {
                    let s = &self.settings;
                    let speech = render(&s.gift_template, &nickname, &handle, "", &gift, count);
                    let outcome = if history {
                        Outcome::History
                    } else if !s.read_gifts {
                        Outcome::Skipped("regalos desactivados")
                    } else if diamonds > 0 && diamonds < s.min_gift_diamonds {
                        Outcome::Skipped("regalo bajo el mínimo")
                    } else {
                        Outcome::Spoken
                    };
                    let text = format!("{gift} x{count}");
                    self.push(Kind::Gift, nickname, handle, text, speech, outcome);
                }
                LiveEvent::Follow { nickname, handle, history } => {
                    let s = &self.settings;
                    let speech = render(&s.follow_template, &nickname, &handle, "", "", 0);
                    let outcome = simple_outcome(history, s.read_follows);
                    self.push(Kind::Follow, nickname, handle, "te sigue".into(), speech, outcome);
                }
                LiveEvent::Share { nickname, handle, history } => {
                    let s = &self.settings;
                    let speech = render(&s.share_template, &nickname, &handle, "", "", 0);
                    let outcome = simple_outcome(history, s.read_shares);
                    self.push(Kind::Share, nickname, handle, "compartió".into(), speech, outcome);
                }
            }
        }
    }

    fn on_comment(&mut self, nickname: String, handle: String, text: String, history: bool) {
        let s = &self.settings;
        let cleaned = filter::clean_comment(&text, &[&handle, &nickname], &s.filter);
        let speech = render(
            &s.comment_template,
            &nickname,
            &handle,
            cleaned.as_deref().unwrap_or(&text),
            "",
            0,
        );
        let outcome = if history {
            Outcome::History
        } else if !s.read_comments {
            Outcome::Skipped("comentarios desactivados")
        } else if cleaned.is_none() {
            Outcome::Skipped("filtrado")
        } else if s.skip_repeats && !self.repeats.check(&handle, &text, Instant::now()) {
            Outcome::Skipped("repetido")
        } else {
            Outcome::Spoken
        };
        self.push(Kind::Comment, nickname, handle, text, speech, outcome);
    }

    fn push(
        &mut self,
        kind: Kind,
        nickname: String,
        handle: String,
        text: String,
        speech: String,
        outcome: Outcome,
    ) {
        if outcome == Outcome::Spoken {
            if let Some(sp) = &self.speaker {
                sp.enqueue(speech.clone());
            }
        }
        if self.feed.len() == FEED_CAP {
            self.feed.pop_front();
        }
        self.feed.push_back(FeedItem { kind, nickname, handle, text, speech, outcome });
    }

    // ---------------------------------------------------------------- UI

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("TikVoz").strong());
            ui.separator();
            ui.label("@");
            let edit = ui.add_enabled(
                !self.is_active(),
                egui::TextEdit::singleline(&mut self.settings.username)
                    .hint_text("usuario de TikTok")
                    .desired_width(180.0),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if self.is_active() {
                if ui.button("⏹ Desconectar").clicked() {
                    self.live.disconnect();
                }
            } else if ui.button("▶ Conectar").clicked() || enter {
                self.connect();
            }
            ui.separator();
            let (color, text) = match &self.status {
                Status::Idle => (Color32::GRAY, "Desconectado".to_string()),
                Status::Connecting(t) => (Color32::from_rgb(230, 180, 40), t.clone()),
                Status::Live(t) => (Color32::from_rgb(60, 200, 90), t.clone()),
                Status::Waiting(t) => (Color32::from_rgb(230, 150, 40), t.clone()),
                Status::Error(t) => (Color32::from_rgb(230, 70, 70), t.clone()),
            };
            ui.label(RichText::new("●").color(color));
            let label = ui.label(text);
            if let Some(room) = self.room_id {
                label.on_hover_text(format!("Sala {room}"));
            }
        });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let Some(sp) = self.speaker.clone() else {
            ui.colored_label(
                Color32::from_rgb(230, 70, 70),
                format!(
                    "Motor de voz no disponible: {}",
                    self.engine_error.as_deref().unwrap_or("desconocido")
                ),
            );
            return;
        };
        ui.horizontal(|ui| {
            let label = if self.paused { "▶ Reanudar" } else { "⏸ Pausar" };
            if ui.button(label).clicked() {
                self.paused = !self.paused;
                sp.set_paused(self.paused);
            }
            if ui.button("⏭ Saltar").on_hover_text("Corta el mensaje actual").clicked() {
                sp.skip();
            }
            if ui.button("🗑 Vaciar cola").clicked() {
                sp.clear();
            }
            ui.separator();
            ui.label(format!(
                "En cola: {}   Leídos: {}   Descartados por cola llena: {}",
                sp.shared.queued(),
                sp.shared.spoken(),
                sp.shared.dropped()
            ));
        });
        let now = sp
            .shared
            .current()
            .map(|t| format!("🔊 {t}"))
            .unwrap_or_else(|| if self.paused { "⏸ En pausa".into() } else { "🔇 En silencio".into() });
        ui.add(egui::Label::new(RichText::new(now).italics()).truncate());
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::CollapsingHeader::new("🔊 Voz").default_open(true).show(ui, |ui| {
                self.voice_settings(ui);
            });
            egui::CollapsingHeader::new("📢 Qué leer").default_open(true).show(ui, |ui| {
                self.read_settings(ui);
            });
            egui::CollapsingHeader::new("🧹 Filtros").default_open(false).show(ui, |ui| {
                self.filter_settings(ui);
            });
            egui::CollapsingHeader::new("⚙ Avanzado").default_open(false).show(ui, |ui| {
                self.advanced_settings(ui);
            });
        });
    }

    fn voice_settings(&mut self, ui: &mut egui::Ui) {
        let Some(engine) = &self.engine else {
            ui.label("Sin motor de voz.");
            return;
        };
        let mut voices: Vec<_> = engine.voices.iter().collect();
        voices.sort_by_key(|v| (!v.language.starts_with("es"), v.name.clone()));
        let selected = voices
            .iter()
            .find(|v| Some(&v.id) == self.settings.voice_id.as_ref())
            .map(|v| format!("{} ({})", v.name, v.language))
            .unwrap_or_else(|| "Predeterminada".into());
        egui::ComboBox::from_id_salt("voz")
            .selected_text(selected)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                for v in voices {
                    ui.selectable_value(
                        &mut self.settings.voice_id,
                        Some(v.id.clone()),
                        format!("{} ({})", v.name, v.language),
                    );
                }
            });
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut self.settings.rate, engine.min_rate..=engine.max_rate.min(3.0))
                    .text("Velocidad"),
            );
            if ui.small_button("↺").on_hover_text("Velocidad normal").clicked() {
                self.settings.rate = engine.normal_rate;
            }
        });
        ui.add(
            egui::Slider::new(&mut self.settings.volume, engine.min_volume..=engine.max_volume)
                .text("Volumen"),
        );
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.test_text).desired_width(ui.available_width() - 70.0));
            if ui.button("Probar").clicked() {
                if let Some(sp) = &self.speaker {
                    sp.say_now(self.test_text.clone());
                }
            }
        });
        ui.small("Instala más voces en Configuración de Windows → Hora e idioma → Voz.");
    }

    fn read_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.checkbox(&mut s.read_comments, "Comentarios");
        template_edit(ui, &mut s.comment_template, "{usuario} {mensaje}");
        ui.checkbox(&mut s.read_gifts, "Regalos");
        template_edit(ui, &mut s.gift_template, "{usuario} {cantidad} {regalo}");
        ui.horizontal(|ui| {
            ui.label("Mínimo de diamantes:");
            ui.add(egui::DragValue::new(&mut s.min_gift_diamonds).range(0..=10_000));
        });
        ui.checkbox(&mut s.read_follows, "Nuevos seguidores");
        template_edit(ui, &mut s.follow_template, "{usuario}");
        ui.checkbox(&mut s.read_shares, "Compartidos");
        template_edit(ui, &mut s.share_template, "{usuario}");
    }

    fn filter_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.checkbox(&mut s.skip_repeats, "No repetir el mismo mensaje del mismo usuario");
        ui.add_enabled(
            s.skip_repeats,
            egui::Slider::new(&mut s.repeat_window_secs, 5..=600).text("seg. de memoria"),
        );
        ui.checkbox(&mut s.filter.strip_links, "Reemplazar links por \"enlace\"");
        ui.checkbox(&mut s.filter.skip_commands, "Ignorar comandos (!algo)");
        ui.add(egui::Slider::new(&mut s.filter.max_chars, 20..=500).text("máx. caracteres"));
        ui.add(egui::Slider::new(&mut s.filter.min_chars, 1..=20).text("mín. letras"));
        ui.label("Palabras bloqueadas (separadas por coma):");
        ui.add(egui::TextEdit::multiline(&mut s.filter.blocked_words).desired_rows(2));
        ui.label("Usuarios silenciados (separados por coma):");
        ui.add(egui::TextEdit::multiline(&mut s.filter.blocked_users).desired_rows(2));
    }

    fn advanced_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.add(egui::Slider::new(&mut s.max_queue, 1..=100).text("máx. en cola"))
            .on_hover_text("Si se llena, se descartan los más viejos: así nunca vas atrasado.");
        ui.checkbox(&mut s.auto_connect, "Conectar al abrir la app");
        ui.label("API key de Euler Stream (opcional):");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut s.api_key)
                    .password(!self.show_api_key)
                    .desired_width(ui.available_width() - 34.0),
            );
            if ui.button("👁").clicked() {
                self.show_api_key = !self.show_api_key;
            }
        });
        ui.hyperlink_to("Conseguir una gratis", "https://www.eulerstream.com/");
    }

    fn feed_panel(&mut self, ui: &mut egui::Ui) {
        if self.feed.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Escribe tu @usuario arriba y pulsa Conectar.\nLos comentarios aparecerán aquí.")
                        .weak(),
                );
            });
            return;
        }
        let mut to_speak = None;
        let mut to_block = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (i, item) in self.feed.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button("🔊").on_hover_text("Leer ahora").clicked() {
                            to_speak = Some(item.speech.clone());
                        }
                        if ui.small_button("🚫").on_hover_text("Silenciar usuario").clicked() {
                            to_block = Some(item.handle.clone());
                        }
                        let dim = item.outcome != Outcome::Spoken;
                        let name = RichText::new(display_name(&item.nickname, &item.handle)).strong();
                        ui.label(item.kind.icon());
                        ui.label(if dim { name.weak() } else { name.color(Color32::from_rgb(120, 180, 255)) });
                        let text = RichText::new(&item.text);
                        ui.label(if dim { text.weak() } else { text });
                        match &item.outcome {
                            Outcome::Skipped(why) => {
                                ui.label(RichText::new(format!("({why})")).small().weak());
                            }
                            Outcome::History => {
                                ui.label(RichText::new("(anterior)").small().weak());
                            }
                            Outcome::Spoken => {}
                        }
                    })
                    .response
                    .on_hover_text(format!("@{} · #{i}", item.handle));
                }
            });
        if let (Some(text), Some(sp)) = (to_speak, &self.speaker) {
            sp.say_now(text);
        }
        if let Some(handle) = to_block.filter(|h| !h.is_empty()) {
            let list = &mut self.settings.filter.blocked_users;
            if !list.split(',').any(|u| u.trim().eq_ignore_ascii_case(&handle)) {
                if !list.trim().is_empty() {
                    list.push_str(", ");
                }
                list.push_str(&handle);
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::TopBottomPanel::bottom("bottom").show(ctx, |ui| {
            ui.add_space(4.0);
            self.bottom_bar(ui);
            ui.add_space(2.0);
        });
        egui::SidePanel::left("settings")
            .resizable(true)
            .default_width(320.0)
            .min_width(260.0)
            .show(ctx, |ui| self.settings_panel(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.feed_panel(ui));

        if self.settings != self.applied {
            self.push_voice_settings(false);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}

fn simple_outcome(history: bool, enabled: bool) -> Outcome {
    if history {
        Outcome::History
    } else if enabled {
        Outcome::Spoken
    } else {
        Outcome::Skipped("desactivado")
    }
}

fn display_name(nickname: &str, handle: &str) -> String {
    if nickname.trim().is_empty() {
        handle.to_string()
    } else {
        nickname.to_string()
    }
}

/// Rellena una plantilla de lectura. El nombre se limpia (sin emojis); si queda
/// vacío se usa el @handle.
fn render(template: &str, nickname: &str, handle: &str, message: &str, gift: &str, count: u32) -> String {
    let mut name = filter::clean_name(nickname);
    if name.is_empty() {
        name = filter::clean_name(handle);
    }
    let out = template
        .replace("{regalo}", gift)
        .replace("{cantidad}", &count.to_string());
    filter::render_template(&out, &name, message)
}

fn template_edit(ui: &mut egui::Ui, template: &mut String, vars: &str) {
    ui.indent(vars, |ui| {
        ui.add(egui::TextEdit::singleline(template).desired_width(f32::INFINITY))
            .on_hover_text(format!("Variables: {vars}"));
    });
}
