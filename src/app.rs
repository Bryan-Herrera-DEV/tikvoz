//! Interfaz (egui, modo inmediato). Solo se redibuja cuando llega un evento o el
//! usuario interactúa, así que en reposo el consumo de CPU es prácticamente cero.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::config::{Settings, Theme};
use crate::demo;
use crate::filter::{self, RepeatGuard};
use crate::kick::KickClient;
use crate::live::{Emit, LiveEvent, Platform, Status, Viewer};
use crate::monitor::Monitor;
use crate::profanity::{self, Profanity};
use crate::speech::{EngineInfo, Speaker, Utterance};
use crate::tiktok::LiveClient;

const FEED_CAP: usize = 300;
const CENSOR: &str = "piip";

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Comment,
    Gift,
    Follow,
    Share,
    Join,
    Sub,
}

impl Kind {
    fn icon(self) -> &'static str {
        match self {
            Kind::Comment => "💬",
            Kind::Gift => "🎁",
            Kind::Follow => "➕",
            Kind::Share => "🔗",
            Kind::Join => "👋",
            Kind::Sub => "💎",
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
    user: Viewer,
    vip: bool,
    text: String,
    /// Lo que se leería si el usuario pulsa 🔊.
    speech: String,
    outcome: Outcome,
}

pub struct App {
    settings: Settings,
    applied: Settings,
    tiktok: LiveClient,
    kick: KickClient,
    events: Receiver<LiveEvent>,
    speaker: Option<Speaker>,
    engine: Option<EngineInfo>,
    engine_error: Option<String>,
    /// Estado por plataforma (índice = `Platform::index`).
    status: [Status; 3],
    feed: VecDeque<FeedItem>,
    repeats: RepeatGuard,
    /// Último momento en que se leyó a cada usuario (cooldown).
    last_spoken: HashMap<String, Instant>,
    /// Groserías acumuladas por usuario (para el auto-silencio).
    strikes: HashMap<String, u32>,
    profanity: Profanity,
    /// Texto de la caja "probar filtro".
    profanity_test: String,
    /// Emisor de eventos (lo comparte el modo demo).
    emit: Emit,
    demo: Option<demo::DemoHandle>,
    paused: bool,
    monitor: Option<Monitor>,
    test_text: String,
    show_api_key: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fallback_fonts(&cc.egui_ctx);
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        settings.migrate();

        let ctx = cc.egui_ctx.clone();
        let (tx, events) = mpsc::channel();
        let repaint = ctx.clone();
        let emit: Emit = Arc::new(move |e| {
            let _ = tx.send(e);
            repaint.request_repaint();
        });
        let tiktok = LiveClient::spawn(emit.clone()).expect("no se pudo iniciar el hilo de TikTok");
        let kick = KickClient::spawn(emit.clone()).expect("no se pudo iniciar el hilo de Kick");

        let repaint = ctx.clone();
        let (speaker, engine, engine_error) =
            match Speaker::spawn(settings.max_queue, move || repaint.request_repaint()) {
                Ok((s, info)) => (Some(s), Some(info), None),
                Err(e) => (None, None, Some(e.to_string())),
            };

        let mut app = Self {
            repeats: RepeatGuard::new(Duration::from_secs(settings.repeat_window_secs)),
            last_spoken: HashMap::new(),
            strikes: HashMap::new(),
            profanity: build_profanity(&settings),
            profanity_test: String::new(),
            emit,
            demo: None,
            applied: settings.clone(),
            settings,
            tiktok,
            kick,
            events,
            speaker,
            engine,
            engine_error,
            status: [Status::Idle, Status::Idle, Status::Idle],
            feed: VecDeque::with_capacity(FEED_CAP),
            paused: false,
            monitor: None,
            test_text: "Hola, así sonarán los comentarios de tu live.".into(),
            show_api_key: false,
        };
        app.apply_theme(&cc.egui_ctx);
        app.pick_default_voice();
        app.push_voice_settings(true);
        app.handle_cli();
        app
    }

    /// `tikvoz.exe usuario` · `--kick canal` · `--demo` · `--config carpeta` (lo lee main.rs)
    fn handle_cli(&mut self) {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut connect_tiktok = self.settings.auto_connect;
        let mut connect_kick = self.settings.auto_connect;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--demo" => self.start_demo(),
                "--config" => {
                    it.next();
                }
                "--kick" => {
                    if let Some(ch) = it.next() {
                        self.settings.kick_channel = ch.clone();
                        connect_kick = true;
                    }
                }
                user if !user.trim().is_empty() => {
                    self.settings.username = user.to_string();
                    connect_tiktok = true;
                }
                _ => {}
            }
        }
        if connect_tiktok && !self.settings.username.trim().is_empty() {
            self.connect(Platform::TikTok);
        }
        if connect_kick && !self.settings.kick_channel.trim().is_empty() {
            self.connect(Platform::Kick);
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            Theme::Sistema => egui::ThemePreference::System,
            Theme::Oscuro => egui::ThemePreference::Dark,
            Theme::Claro => egui::ThemePreference::Light,
        });
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

    /// Aplica solo lo que cambió (o todo si `force`).
    fn push_voice_settings(&mut self, force: bool) {
        let (s, a) = (&self.settings, &self.applied);
        if force
            || s.filter.bad_words != a.filter.bad_words
            || s.filter.allowed_words != a.filter.allowed_words
            || s.filter.disabled_categories != a.filter.disabled_categories
        {
            self.profanity = build_profanity(s);
        }
        if force || s.repeat_window_secs != a.repeat_window_secs {
            self.repeats.set_window(Duration::from_secs(s.repeat_window_secs));
        }
        if let Some(sp) = &self.speaker {
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
        }
        self.applied = self.settings.clone();
    }

    fn connect(&mut self, platform: Platform) {
        match platform {
            Platform::TikTok => {
                let user = self.settings.username.trim().trim_start_matches('@').to_string();
                if !user.is_empty() {
                    self.settings.username = user.clone();
                    self.tiktok.connect(&user, self.settings.api_key());
                }
            }
            Platform::Kick => {
                let slug = crate::kick::normalize_slug(&self.settings.kick_channel);
                if !slug.is_empty() {
                    self.settings.kick_channel = slug.clone();
                    self.kick.connect(&slug);
                }
            }
            Platform::Demo => self.start_demo(),
        }
    }

    fn disconnect(&mut self, platform: Platform) {
        match platform {
            Platform::TikTok => self.tiktok.disconnect(),
            Platform::Kick => self.kick.disconnect(),
            Platform::Demo => {
                self.demo = None;
                self.status[Platform::Demo.index()] = Status::Idle;
            }
        }
    }

    fn start_demo(&mut self) {
        self.demo = Some(demo::spawn(self.emit.clone()));
    }

    fn is_active(&self, platform: Platform) -> bool {
        self.status[platform.index()].is_active()
    }

    // ------------------------------------------------------------ eventos

    fn drain_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                LiveEvent::Status(p, s) => self.status[p.index()] = s,
                LiveEvent::Comment { user, text, history } => self.on_comment(user, text, history),
                LiveEvent::Gift { user, gift, count, diamonds, history } => {
                    let s = &self.settings;
                    let speech = self.render(&s.gift_template, &user, "", &gift, count);
                    let outcome = if history {
                        Outcome::History
                    } else if !s.read_gifts {
                        Outcome::Skipped("regalos desactivados")
                    } else if diamonds > 0 && diamonds < s.min_gift_diamonds {
                        Outcome::Skipped("regalo bajo el mínimo")
                    } else {
                        Outcome::Spoken
                    };
                    let priority = s.priority_gifts;
                    self.push(Kind::Gift, user, format!("{gift} x{count}"), speech, outcome, priority);
                }
                LiveEvent::Follow { user, history } => {
                    let s = &self.settings;
                    let speech = self.render(&s.follow_template, &user, "", "", 0);
                    let outcome = simple_outcome(history, s.read_follows);
                    let priority = s.priority_gifts;
                    self.push(Kind::Follow, user, "te sigue".into(), speech, outcome, priority);
                }
                LiveEvent::Share { user, history } => {
                    let s = &self.settings;
                    let speech = self.render(&s.share_template, &user, "", "", 0);
                    let outcome = simple_outcome(history, s.read_shares);
                    self.push(Kind::Share, user, "compartió".into(), speech, outcome, false);
                }
                LiveEvent::Subscription { user, months, gifted } => {
                    let s = &self.settings;
                    let (template, text, count) = if gifted > 0 {
                        (&s.gifted_sub_template, format!("regaló {gifted} subs"), gifted)
                    } else {
                        (&s.sub_template, format!("se suscribió ({months} meses)"), months)
                    };
                    let speech = self.render(template, &user, "", "", count);
                    let outcome = simple_outcome(false, s.read_subs);
                    let priority = s.priority_gifts;
                    self.push(Kind::Sub, user, text, speech, outcome, priority);
                }
                LiveEvent::Join { user } => {
                    let s = &self.settings;
                    let speech = self.render(&s.join_template, &user, "", "", 0);
                    // Las entradas son muchísimas: solo se leen si no hay nada esperando.
                    let idle = self
                        .speaker
                        .as_ref()
                        .is_some_and(|sp| sp.shared.queued() == 0 && sp.shared.current().is_none());
                    let outcome = if !s.read_joins {
                        Outcome::Skipped("desactivado")
                    } else if !idle {
                        Outcome::Skipped("cola ocupada")
                    } else {
                        Outcome::Spoken
                    };
                    if s.show_joins || outcome == Outcome::Spoken {
                        self.push(Kind::Join, user, "se unió".into(), speech, outcome, false);
                    }
                }
            }
        }
    }

    fn is_vip(&self, user: &Viewer) -> bool {
        self.settings
            .vip_users
            .split(',')
            .map(|u| u.trim().trim_start_matches('@'))
            .any(|u| {
                !u.is_empty() && (u.eq_ignore_ascii_case(&user.handle) || u.eq_ignore_ascii_case(&user.nickname))
            })
    }

    /// `true` si el usuario puede hablar ya (y registra el momento).
    fn cooldown_ok(&mut self, key: &str, now: Instant) -> bool {
        let secs = self.settings.user_cooldown_secs;
        if secs == 0 {
            return true;
        }
        let window = Duration::from_secs(secs);
        if self.last_spoken.len() > 512 {
            self.last_spoken.retain(|_, t| now.duration_since(*t) < window);
        }
        match self.last_spoken.get(key) {
            Some(t) if now.duration_since(*t) < window => false,
            _ => {
                self.last_spoken.insert(key.to_string(), now);
                true
            }
        }
    }

    /// Suma una falta; al llegar al límite silencia al usuario. Devuelve `true` si lo silenció.
    fn strike(&mut self, user: &Viewer) -> bool {
        let limit = self.settings.filter.auto_ban_strikes;
        if limit == 0 || user.handle.is_empty() {
            return false;
        }
        let key = format!("{}:{}", user.platform.name(), user.handle.to_lowercase());
        let n = self.strikes.entry(key).or_default();
        *n += 1;
        if *n >= limit {
            add_to_list(&mut self.settings.filter.blocked_users, &user.handle);
            return true;
        }
        false
    }

    fn on_comment(&mut self, user: Viewer, text: String, history: bool) {
        let now = Instant::now();
        let privileged = self.is_vip(&user) || (self.settings.mods_priority && user.moderator);

        // 1) Groserías: aplica a todas las plataformas, VIP y mods incluidos.
        let hits = if history { Vec::new() } else { self.profanity.find(&text) };
        let has_bad = !hits.is_empty();
        let shown = if has_bad { self.profanity.censor(&text, CENSOR) } else { text.clone() };
        let banned_now = has_bad && self.strike(&user);

        let s = &self.settings;
        let source = if has_bad && s.filter.censor_instead_of_skip { &shown } else { &text };
        let cleaned = filter::clean_comment(source, &[&user.handle, &user.nickname], &s.filter);
        let speech = self.render(&s.comment_template, &user, cleaned.as_deref().unwrap_or(&shown), "", 0);
        let cooldown_key = format!("{}:{}", user.platform.name(), user.handle);

        let outcome = if history {
            Outcome::History
        } else if banned_now {
            Outcome::Skipped("grosería · usuario silenciado")
        } else if has_bad && !s.filter.censor_instead_of_skip {
            Outcome::Skipped("grosería")
        } else if !s.read_comments {
            Outcome::Skipped("comentarios desactivados")
        } else if s.only_followers && user.follower == Some(false) && !privileged {
            Outcome::Skipped("no te sigue")
        } else if cleaned.is_none() {
            Outcome::Skipped("filtrado")
        } else if !privileged && s.skip_repeats && !self.repeats.check(&cooldown_key, &text, now) {
            Outcome::Skipped("repetido")
        } else if !privileged && !self.cooldown_ok(&cooldown_key, now) {
            Outcome::Skipped("en enfriamiento")
        } else {
            Outcome::Spoken
        };
        // En el feed nunca mostramos la grosería sin censurar (puede estar en pantalla).
        self.push(Kind::Comment, user, shown, speech, outcome, privileged);
    }

    /// Voz estable por usuario: misma voz siempre para el mismo @handle, elegida
    /// entre las voces del mismo idioma que la principal.
    fn voice_for(&self, handle: &str) -> Option<String> {
        if !self.settings.voice_per_user {
            return None;
        }
        let engine = self.engine.as_ref()?;
        let main = self.settings.voice_id.as_ref()?;
        let lang = engine.voices.iter().find(|v| &v.id == main)?.language.get(..2)?.to_string();
        let pool: Vec<&String> = engine
            .voices
            .iter()
            .filter(|v| v.language.starts_with(&lang))
            .map(|v| &v.id)
            .collect();
        if pool.len() < 2 {
            return None;
        }
        // FNV-1a: estable entre ejecuciones (a diferencia del hasher de std).
        let h = handle
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
        Some(pool[(h % pool.len() as u64) as usize].clone())
    }

    /// Nombre que dirá la voz: limpio de emojis y sin groserías.
    fn spoken_name(&self, user: &Viewer) -> String {
        let censor = self.settings.filter.censor_names;
        [&user.nickname, &user.handle]
            .into_iter()
            .map(|n| filter::clean_name(n))
            .find(|n| !(n.is_empty() || censor && self.profanity.contains(n)))
            .unwrap_or_else(|| "alguien".into())
    }

    /// Rellena una plantilla de lectura.
    fn render(&self, template: &str, user: &Viewer, message: &str, gift: &str, count: u32) -> String {
        let out = template
            .replace("{regalo}", gift)
            .replace("{cantidad}", &count.to_string());
        filter::render_template(&out, &self.spoken_name(user), message)
    }

    fn push(&mut self, kind: Kind, user: Viewer, text: String, speech: String, outcome: Outcome, priority: bool) {
        if outcome == Outcome::Spoken {
            if let Some(sp) = &self.speaker {
                let voice = self.voice_for(&user.handle);
                sp.enqueue(Utterance { text: speech.clone(), voice, priority });
            }
        }
        if self.feed.len() == FEED_CAP {
            self.feed.pop_front();
        }
        let vip = self.is_vip(&user);
        self.feed.push_back(FeedItem { kind, user, vip, text, speech, outcome });
    }

    // ---------------------------------------------------------------- UI

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("TikVoz").strong());
            ui.separator();
            self.platform_row(ui, Platform::TikTok);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let t = &mut self.settings.theme;
                ui.selectable_value(t, Theme::Claro, "☀").on_hover_text("Tema claro");
                ui.selectable_value(t, Theme::Oscuro, "🌙").on_hover_text("Tema oscuro");
                ui.selectable_value(t, Theme::Sistema, "💻").on_hover_text("Seguir al sistema");
            });
        });
        ui.horizontal(|ui| {
            // Alinea con la fila de TikTok (ancho aproximado del título).
            ui.add_space(78.0);
            self.platform_row(ui, Platform::Kick);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.is_active(Platform::Demo) {
                    if ui.button("⏹ Demo").clicked() {
                        self.disconnect(Platform::Demo);
                    }
                } else if ui
                    .button("🎭 Demo")
                    .on_hover_text("Simula un live (TikTok + Kick) para probar voces y filtros")
                    .clicked()
                {
                    self.start_demo();
                }
            });
        });
    }

    fn platform_row(&mut self, ui: &mut egui::Ui, platform: Platform) {
        ui.add_sized([58.0, 18.0], egui::Label::new(platform_badge(platform).strong()));
        let active = self.is_active(platform);
        let (field, hint) = match platform {
            Platform::Kick => (&mut self.settings.kick_channel, "canal de Kick"),
            _ => (&mut self.settings.username, "@usuario de TikTok"),
        };
        let edit = ui.add_enabled(!active, egui::TextEdit::singleline(field).hint_text(hint).desired_width(170.0));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if active {
            if ui.button("⏹ Desconectar").clicked() {
                self.disconnect(platform);
            }
        } else if ui.button("▶ Conectar").clicked() || enter {
            self.connect(platform);
        }
        status_label(ui, &self.status[platform.index()]);
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let Some(sp) = self.speaker.clone() else {
            ui.colored_label(
                ui.visuals().error_fg_color,
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
            if self.status[Platform::Demo.index()].is_active() {
                ui.separator();
                status_label(ui, &self.status[Platform::Demo.index()]);
            }
        });
        let now = sp
            .shared
            .current()
            .map(|t| format!("🔊 {t}"))
            .unwrap_or_else(|| if self.paused { "⏸ En pausa".into() } else { "🔇 En silencio".into() });
        ui.horizontal(|ui| {
            self.monitor_label(ui);
            ui.add(egui::Label::new(RichText::new(now).italics()).truncate());
        });
    }

    /// Consumo del propio programa (esquina inferior izquierda).
    fn monitor_label(&self, ui: &mut egui::Ui) {
        let Some(m) = &self.monitor else { return };
        let text = match m.snapshot() {
            None => "CPU –  RAM –".to_string(),
            Some(s) => {
                let mut t = format!("CPU {:.1}%  RAM {:.0} MB", s.cpu_percent, s.ram_mb);
                if let (Some(g), Some(v)) = (s.gpu_percent, s.vram_mb) {
                    t.push_str(&format!("  GPU {g:.0}%  VRAM {v:.0} MB"));
                }
                t
            }
        };
        ui.label(RichText::new(text).monospace().weak())
            .on_hover_text("Consumo solo de TikVoz. RAM = memoria privada.");
        ui.separator();
    }

    /// Arranca o detiene el hilo del monitor según el ajuste.
    fn sync_monitor(&mut self, ctx: &egui::Context) {
        match (self.settings.show_monitor, self.monitor.is_some()) {
            (true, false) => {
                let ctx = ctx.clone();
                self.monitor = Some(Monitor::start(move || ctx.request_repaint()));
            }
            (false, true) => self.monitor = None,
            _ => {}
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::CollapsingHeader::new("🔊 Voz").default_open(true).show(ui, |ui| {
                self.voice_settings(ui);
            });
            egui::CollapsingHeader::new("💬 Qué leer").default_open(true).show(ui, |ui| {
                self.read_settings(ui);
            });
            egui::CollapsingHeader::new("⭐ Audiencia y prioridad").default_open(false).show(ui, |ui| {
                self.audience_settings(ui);
            });
            egui::CollapsingHeader::new("⚠ Groserías").default_open(false).show(ui, |ui| {
                self.profanity_settings(ui);
            });
            egui::CollapsingHeader::new("🚫 Filtros").default_open(false).show(ui, |ui| {
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
        ui.checkbox(&mut s.read_gifts, "Regalos (TikTok) y Kicks (Kick)");
        template_edit(ui, &mut s.gift_template, "{usuario} {cantidad} {regalo}");
        ui.horizontal(|ui| {
            ui.label("Mínimo de diamantes:");
            ui.add(egui::DragValue::new(&mut s.min_gift_diamonds).range(0..=10_000));
        });
        ui.checkbox(&mut s.read_subs, "Suscripciones (Kick)");
        template_edit(ui, &mut s.sub_template, "{usuario} {cantidad}=meses");
        template_edit(ui, &mut s.gifted_sub_template, "{usuario} {cantidad}");
        ui.checkbox(&mut s.read_follows, "Nuevos seguidores (TikTok)");
        template_edit(ui, &mut s.follow_template, "{usuario}");
        ui.checkbox(&mut s.read_shares, "Compartidos (TikTok)");
        template_edit(ui, &mut s.share_template, "{usuario}");
        ui.checkbox(&mut s.read_joins, "Bienvenidas (solo si la cola está vacía)");
        template_edit(ui, &mut s.join_template, "{usuario}");
        ui.checkbox(&mut s.show_joins, "Mostrar entradas en el feed");
    }

    fn audience_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.checkbox(&mut s.voice_per_user, "Voz distinta por usuario")
            .on_hover_text("Cada @usuario suena siempre con la misma voz (del mismo idioma que la principal).");
        ui.checkbox(&mut s.priority_gifts, "Regalos, subs y seguidores se adelantan en la cola");
        ui.checkbox(&mut s.mods_priority, "Moderadores con prioridad y sin límites");
        ui.checkbox(&mut s.only_followers, "Leer solo a quienes te siguen (TikTok)");
        ui.horizontal(|ui| {
            ui.label("Enfriamiento por usuario:");
            ui.add(egui::DragValue::new(&mut s.user_cooldown_secs).range(0..=600).suffix(" s"));
        })
        .response
        .on_hover_text("Tiempo mínimo entre dos lecturas del mismo usuario (0 = sin límite).");
        ui.label("Usuarios VIP (siempre se leen, con prioridad):");
        ui.add(
            egui::TextEdit::multiline(&mut s.vip_users)
                .hint_text("usuario1, usuario2")
                .desired_rows(2),
        );
    }

    fn profanity_settings(&mut self, ui: &mut egui::Ui) {
        ui.small(
            "Detecta trucos como \"pi to\", \"p1to\", \"4buso\", \"a.buso\", \"p*ta\", letras raras \
             (𝓹𝓲𝓽𝓸), cirílicas o escritura fonética (berga, kabron). Aplica a TikTok y Kick.",
        );
        let s = &mut self.settings;
        ui.checkbox(&mut s.filter.censor_instead_of_skip, "Leer el mensaje con \"piip\" en vez de omitirlo");
        ui.checkbox(&mut s.filter.censor_names, "Si el nombre es una grosería, decir \"alguien\"");
        ui.horizontal(|ui| {
            ui.label("Silenciar al usuario tras");
            ui.add(egui::DragValue::new(&mut s.filter.auto_ban_strikes).range(0..=20));
            ui.label("groserías (0 = nunca)");
        });
        let cats = profanity::categories(&s.filter.bad_words);
        if !cats.is_empty() {
            ui.label("Categorías activas:");
            ui.horizontal_wrapped(|ui| {
                for cat in cats {
                    let mut on = !s.filter.disabled_categories.contains(&cat);
                    if ui.checkbox(&mut on, &cat).changed() {
                        if on {
                            s.filter.disabled_categories.retain(|c| c != &cat);
                        } else {
                            s.filter.disabled_categories.push(cat);
                        }
                    }
                }
            });
        }
        ui.label("Lista ([categoría] agrupa; palabra* empieza, *palabra termina, *palabra* contiene):")
            .on_hover_text("Una palabra por línea o separadas por coma. Las frases con espacios también valen.");
        ui.add(
            egui::TextEdit::multiline(&mut s.filter.bad_words)
                .code_editor()
                .desired_rows(5)
                .desired_width(f32::INFINITY),
        );
        if ui.small_button("Restaurar lista por defecto").clicked() {
            s.filter.bad_words = profanity::DEFAULT_BAD_WORDS.into();
        }
        ui.label("Excepciones (nunca cuentan como grosería):");
        ui.add(
            egui::TextEdit::multiline(&mut s.filter.allowed_words)
                .hint_text("p. ej. marica, si en tu comunidad se usa de cariño")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        ui.separator();
        ui.label("Probar el filtro:");
        ui.add(
            egui::TextEdit::singleline(&mut self.profanity_test)
                .hint_text("escribe algo como p1to o a.buso")
                .desired_width(f32::INFINITY),
        );
        if !self.profanity_test.is_empty() {
            let hits = self.profanity.find(&self.profanity_test);
            if hits.is_empty() {
                ui.colored_label(Color32::from_rgb(40, 170, 80), "✔ limpio");
            } else {
                let rules: Vec<String> = hits.iter().map(|h| format!("{} · {}", h.rule, h.category)).collect();
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!(
                        "✖ {}  ({})",
                        self.profanity.censor(&self.profanity_test, CENSOR),
                        rules.join(", ")
                    ),
                );
            }
        }
        if !self.strikes.is_empty() && ui.small_button("Olvidar faltas de esta sesión").clicked() {
            self.strikes.clear();
        }
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
        ui.horizontal(|ui| {
            ui.label("Solo leer si empieza con:");
            ui.add(
                egui::TextEdit::singleline(&mut s.filter.required_prefix)
                    .hint_text("vacío = todos")
                    .desired_width(70.0),
            );
        });
        ui.add(egui::Slider::new(&mut s.filter.max_chars, 20..=500).text("máx. caracteres"));
        ui.add(egui::Slider::new(&mut s.filter.min_chars, 1..=20).text("mín. letras"));
        ui.label("Usuarios silenciados (separados por coma):");
        ui.add(egui::TextEdit::multiline(&mut s.filter.blocked_users).desired_rows(2));
        ui.label("Diccionario de pronunciación (una regla por línea):");
        ui.add(
            egui::TextEdit::multiline(&mut s.filter.dictionary)
                .hint_text("xd = equis de")
                .code_editor()
                .desired_rows(4),
        );
        if ui.small_button("Restaurar diccionario por defecto").clicked() {
            s.filter.dictionary = filter::DEFAULT_DICTIONARY.into();
        }
    }

    fn advanced_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        ui.add(egui::Slider::new(&mut s.max_queue, 1..=100).text("máx. en cola"))
            .on_hover_text("Si se llena, se descartan los más viejos: así nunca vas atrasado.");
        ui.checkbox(&mut s.auto_connect, "Conectar TikTok y Kick al abrir la app");
        ui.checkbox(&mut s.show_monitor, "Mostrar consumo (CPU, RAM, GPU)");
        ui.label("API key de Euler Stream para TikTok (opcional):");
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
                    RichText::new(
                        "Escribe tu @usuario de TikTok y/o tu canal de Kick arriba y pulsa Conectar.\n\
                         Los mensajes de ambas plataformas aparecerán aquí.",
                    )
                    .weak(),
                );
            });
            return;
        }
        let mut to_speak = None;
        let mut to_block = None;
        let mut to_vip = None;
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
                            to_block = Some(item.user.handle.clone());
                        }
                        if ui.small_button("⭐").on_hover_text("Agregar a VIP").clicked() {
                            to_vip = Some(item.user.handle.clone());
                        }
                        let dim = item.outcome != Outcome::Spoken;
                        ui.label(platform_badge(item.user.platform).small());
                        ui.label(item.kind.icon());
                        if item.vip {
                            ui.label("⭐").on_hover_text("VIP");
                        }
                        if item.user.moderator {
                            ui.label("🛡").on_hover_text("Moderador");
                        }
                        let name = RichText::new(display_name(&item.user.nickname, &item.user.handle)).strong();
                        let accent = ui.visuals().hyperlink_color;
                        ui.label(if dim { name.weak() } else { name.color(accent) });
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
                    .on_hover_text(format!(
                        "{} · @{}{}{} · #{i}",
                        item.user.platform.name(),
                        item.user.handle,
                        if item.user.follower == Some(true) { " · te sigue" } else { "" },
                        if item.user.subscriber { " · suscriptor" } else { "" },
                    ));
                }
            });
        if let (Some(text), Some(sp)) = (to_speak, &self.speaker) {
            sp.say_now(text);
        }
        if let Some(handle) = to_block {
            add_to_list(&mut self.settings.filter.blocked_users, &handle);
        }
        if let Some(handle) = to_vip {
            add_to_list(&mut self.settings.vip_users, &handle);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.sync_monitor(ctx);

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

        if self.settings.theme != self.applied.theme {
            self.apply_theme(ctx);
        }
        if self.settings != self.applied {
            self.push_voice_settings(false);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}

fn build_profanity(s: &Settings) -> Profanity {
    Profanity::with_options(&s.filter.bad_words, &s.filter.allowed_words, &s.filter.disabled_categories)
}

/// Etiqueta de color de cada plataforma.
fn platform_badge(p: Platform) -> RichText {
    let color = match p {
        Platform::TikTok => Color32::from_rgb(254, 44, 85),
        Platform::Kick => Color32::from_rgb(60, 200, 20),
        Platform::Demo => Color32::GRAY,
    };
    RichText::new(p.name()).color(color)
}

fn status_label(ui: &mut egui::Ui, status: &Status) {
    let v = ui.visuals();
    let (color, text) = match status {
        Status::Idle => (v.weak_text_color(), "Desconectado".to_string()),
        Status::Connecting(t) => (v.warn_fg_color, t.clone()),
        Status::Live(t) => (Color32::from_rgb(40, 170, 80), t.clone()),
        Status::Waiting(t) => (v.warn_fg_color, t.clone()),
        Status::Error(t) => (v.error_fg_color, t.clone()),
    };
    ui.label(RichText::new("●").color(color));
    ui.add(egui::Label::new(text).truncate());
}

/// Las fuentes de egui no traen símbolos como ● → ni las letras "fancy" que la
/// gente usa en sus nicks (𝓓, ℜ...). Las tomamos de las fuentes del sistema como
/// respaldo; si no existen (otro SO) se sigue con las de egui.
fn install_fallback_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\seguisym.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    ];
    let mut fonts = egui::FontDefinitions::default();
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let name = path.to_string();
        fonts
            .font_data
            .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    ctx.set_fonts(fonts);
}

/// Agrega `item` a una lista separada por comas si no está ya.
fn add_to_list(list: &mut String, item: &str) {
    if item.is_empty() || list.split(',').any(|u| u.trim().eq_ignore_ascii_case(item)) {
        return;
    }
    if !list.trim().is_empty() {
        list.push_str(", ");
    }
    list.push_str(item);
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

fn template_edit(ui: &mut egui::Ui, template: &mut String, vars: &str) {
    ui.indent(vars, |ui| {
        ui.add(egui::TextEdit::singleline(template).desired_width(f32::INFINITY))
            .on_hover_text(format!("Variables: {vars}"));
    });
}
