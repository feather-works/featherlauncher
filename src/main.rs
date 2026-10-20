#![cfg_attr(windows, windows_subsystem = "windows")]
//! FeatherWorks Launcher: instala, atualiza e abre os aplicativos da suíte.
mod aviso;
mod bandeja;
mod consumo;
mod instalar;
mod releases;
mod tarefas;

use instalar::Evento;
use pelicano::column;
use pelicano::*;
use releases::{mais_nova, App as AppInfo, Release, APPS};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tarefas::no_tokio;

const TAMANHO_INICIAL: Size = Size::new(1440.0, 860.0);
const TAMANHO_MINIMO: Size = Size::new(960.0, 520.0);
const LARGURA_MENU: f32 = 236.0;
const RAIO_JANELA: f32 = 12.0;
const MARGEM: f32 = 16.0;
const ALTURA_CABECALHO: f32 = 98.0;
const ALTURA_RODAPE: f32 = 44.0;
const RAIO_VIDRO: f32 = 14.0;
// Colunas da lista, iguais no cabeçalho e nas linhas.
const COL_VERSAO: f32 = 150.0;
const COL_ESTADO: f32 = 150.0;
const COL_DATA: f32 = 170.0;
const COL_ACOES: f32 = 230.0;
/// O botão principal da linha; com o "⋯" ao lado fecha a coluna de ações.
const LARGURA_BOTAO: f32 = COL_ACOES - 8.0 - 36.0;
const LARANJA: &str = "#b45309";
const VERDE: &str = "#287846";
const AZUL: &str = "#1a56b8";
/// A pena do launcher: magenta, cor que nenhum aplicativo da suíte usa.
const COR_LAUNCHER: &str = "#b8336a";

fn main() -> Result<(), Error> {
    // Sobra da última atualização no Windows (o .exe em uso só pode ser renomeado).
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
    // O que o login abre: o ícone na bandeja, que confere as versões e avisa por notificação.
    if std::env::args().any(|a| a == "--bandeja") {
        bandeja::rodar();
        return Ok(());
    }
    let (tema, grade, perguntou_inicio) = ler_preferencias();
    aplicar_tema(tema);
    let settings = Settings {
        title: "FeatherWorks Launcher".into(),
        size: TAMANHO_INICIAL,
        min_size: Some(TAMANHO_MINIMO),
        // A barra de título é desenhada pelo app; os cantos arredondados pedem a janela transparente.
        decorations: false,
        transparent: true,
        // Alt+F4 passa pelo `Msg::Fechar`, que não fecha no meio de uma instalação.
        close_on_request: false,
        app_id: Some("featherlauncher".into()),
        icon: icone_da_janela(),
        ..Default::default()
    };
    run(settings, move || Launcher::novo(tema, grade, perguntou_inicio))
}

/// A pena na barra de tarefas (Windows e X11; no Wayland o ícone vem do .desktop).
fn icone_da_janela() -> Option<Icon> {
    let img = ::image::load_from_memory(include_bytes!("../pacote/icones/featherlauncher-64.png")).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

enum Consulta {
    Verificando,
    Falhou(String),
    /// `None` = sem release publicada ainda.
    Pronta(Option<Release>),
}

enum Download {
    Baixando { baixado: u64, total: u64, alca: Handle },
    Pausado { baixado: u64, total: u64 },
    Instalando,
    Desinstalando,
}

struct Linha {
    consulta: Consulta,
    /// Versão instalada: `Some("?")` quando o app está lá mas não foi o launcher que instalou.
    instalada: Option<String>,
    download: Option<Download>,
    erro: Option<String>,
}

impl Linha {
    fn release(&self) -> Option<&Release> {
        match &self.consulta {
            Consulta::Pronta(r) => r.as_ref(),
            _ => None,
        }
    }

    fn atualizacao(&self) -> Option<&Release> {
        let rel = self.release().filter(|r| r.pacote.is_some())?;
        let inst = self.instalada.as_deref()?;
        (inst == "?" || mais_nova(&rel.versao, inst)).then_some(rel)
    }

    fn instalavel(&self) -> Option<&Release> {
        self.release().filter(|r| r.pacote.is_some() && self.instalada.is_none())
    }

    fn em_breve(&self) -> bool {
        matches!(self.consulta, Consulta::Pronta(None)) && self.instalada.is_none()
    }
}

/// O tema da janela, guardado em `~/.config/featherworks/launcher.json`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tema {
    Automatico,
    Claro,
    Escuro,
}

impl Tema {
    const TODOS: [(Tema, &'static str, &'static str); 3] = [(Tema::Automatico, "automatico", "Automático"), (Tema::Claro, "claro", "Claro"), (Tema::Escuro, "escuro", "Escuro")];
}

fn arquivo_preferencias() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("featherworks/launcher.json")
}

fn json_preferencias() -> serde_json::Value {
    std::fs::read(arquivo_preferencias()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Tema, modo de exibição (`true` = grade) e se já perguntou sobre abrir com o sistema.
fn ler_preferencias() -> (Tema, bool, bool) {
    let json = json_preferencias();
    (Tema::TODOS.iter().find(|t| json["tema"] == t.1).map_or(Tema::Automatico, |t| t.0), json["modo"] == "grade", json["perguntou_inicio"] == true)
}

/// Muda só as chaves pedidas: as outras preferências ficam como estão no arquivo.
fn alterar_preferencias(mudar: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>)) {
    let caminho = arquivo_preferencias();
    let mut json = match json_preferencias() {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    mudar(&mut json);
    let _ = std::fs::create_dir_all(caminho.parent().unwrap());
    let _ = std::fs::write(caminho, serde_json::Value::Object(json).to_string());
}

fn guardar_preferencias(t: Tema, grade: bool) {
    let nome = Tema::TODOS.iter().find(|x| x.0 == t).map_or("automatico", |x| x.1);
    alterar_preferencias(|j| {
        j.insert("tema".into(), nome.into());
        j.insert("modo".into(), if grade { "grade" } else { "lista" }.into());
    });
}

static ESCURO: AtomicBool = AtomicBool::new(true);

fn escuro() -> bool {
    ESCURO.load(Ordering::Relaxed)
}

fn aplicar_tema(t: Tema) {
    let e = match t {
        Tema::Automatico => sistema_escuro(),
        t => t == Tema::Escuro,
    };
    ESCURO.store(e, Ordering::Relaxed);
}

/// O GNOME guarda o modo escuro em `color-scheme`.
// shortcut: fora do GNOME o automático fica escuro; ler o portal (org.freedesktop.appearance) e o registro do Windows quando alguém pedir.
fn sistema_escuro() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .filter(|s| s.status.success())
        .map_or(true, |s| !String::from_utf8_lossy(&s.stdout).contains("default") && !String::from_utf8_lossy(&s.stdout).contains("light"))
}

// ---------- Cores (as do tema do feather-ui, claro e escuro) ----------

fn hex(s: &str) -> Color {
    Color::hex(s).unwrap_or(Color::BLACK)
}

fn par(claro: &str, escuro_: &str) -> Color {
    hex(if escuro() { escuro_ } else { claro })
}

fn texto() -> Color {
    par("#202124", "#e8eaed")
}
fn mudo() -> Color {
    // No claro, o cinza do feather-ui (#5f6368) some sobre o vidro: um tom mais escuro.
    par("#3c4043", "#9aa0a6")
}
fn borda() -> Color {
    par("#dadce0", "#3d4147")
}
fn borda_suave() -> Color {
    par("#e8eaed", "#33363b")
}
fn painel() -> Color {
    par("#ffffff", "#2a2d32")
}
fn erro() -> Color {
    par("#d93025", "#f28b82")
}
fn acento() -> Color {
    par("#2b7a78", "#4fb3ad")
}

/// O fundo dos painéis: no escuro, um azul-marinho que conversa com o roxo da foto.
fn cor_painel() -> Color {
    if escuro() {
        Color::rgb8(17, 21, 31)
    } else {
        painel()
    }
}

/// As cores de estado legíveis no tema atual: no escuro, os mesmos tons mais claros.
fn tom(cor: &str) -> Color {
    if !escuro() {
        return hex(cor);
    }
    hex(match cor {
        VERDE => "#81c995",
        LARANJA => "#f6ae6b",
        AZUL => "#8ab4f8",
        outra => outra,
    })
}

/// A cor do nome do app, mais clara no escuro para ler sobre o vidro.
fn tom_app(cor: &str) -> Color {
    if escuro() {
        hex(cor).mix(Color::WHITE, 0.25)
    } else {
        hex(cor)
    }
}

/// Preto ou branco, o que contrastar mais com `fundo` (WCAG).
fn texto_sobre(fundo: Color) -> Color {
    let lin = |c: f32| if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let l = 0.2126 * lin(fundo.r) + 0.7152 * lin(fundo.g) + 0.0722 * lin(fundo.b);
    // Contraste com o preto (l + 0.05) / 0.05 contra o branco 1.05 / (l + 0.05).
    if (l + 0.05) * (l + 0.05) >= 0.0525 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// O texto e o ícone sobre o botão de destaque.
fn tinta_destaque() -> Color {
    texto_sobre(hex(COR_LAUNCHER))
}

// ---------- Ícones (Tabler) e a pena ----------

const ICONE_CONFIG: &str = r#"<path d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 0 0-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 0 0-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 0 0-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 0 0-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 0 0 1.066-2.573c-.94-1.543.826-3.31 2.37-2.37 1 .608 2.296.07 2.572-1.065z"/><path d="M9 12a3 3 0 1 0 6 0a3 3 0 0 0-6 0"/>"#;
const ICONE_REFRESH: &str = r#"<path d="M20 11a8.1 8.1 0 0 0-15.5-2m-.5-4v4h4"/><path d="M4 13a8.1 8.1 0 0 0 15.5 2m.5 4v-4h-4"/>"#;
const ICONE_LISTA: &str = r#"<path d="M9 6h11M9 12h11M9 18h11M5 6v.01M5 12v.01M5 18v.01"/>"#;
const ICONE_GRADE: &str = r#"<path d="M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z"/>"#;
const ICONE_BAIXAR: &str = r#"<path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2M7 11l5 5 5-5M12 4v12"/>"#;
const ICONE_MAIS: &str = r#"<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>"#;
const ICONE_BUSCA: &str = r#"<circle cx="10" cy="10" r="7"/><path d="M21 21l-6-6"/>"#;
const ICONE_CADEADO: &str = r#"<path d="M5 13a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2zM11 16a1 1 0 1 0 2 0 1 1 0 0 0-2 0M8 11V7a4 4 0 1 1 8 0v4"/>"#;
const ICONE_RELOGIO: &str = r#"<path d="M3 12a9 9 0 1 0 18 0 9 9 0 0 0-18 0M12 7v5l3 3"/>"#;
const ICONE_PACOTE: &str = r#"<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3M12 12l8-4.5M12 12v9M12 12L4 7.5"/>"#;
const ICONE_GITHUB: &str = r#"<path d="M9 19c-4.3 1.4-4.3-2.5-6-3m12 5v-3.5c0-1 .1-1.4-.5-2 2.8-.3 5.5-1.4 5.5-6a4.6 4.6 0 0 0-1.3-3.2 4.2 4.2 0 0 0-.1-3.2s-1.1-.3-3.5 1.3a12.3 12.3 0 0 0-6.2 0C6.5 2.8 5.4 3.1 5.4 3.1a4.2 4.2 0 0 0-.1 3.2A4.6 4.6 0 0 0 4 9.5c0 4.6 2.7 5.7 5.5 6-.6.6-.6 1.2-.5 2V21"/>"#;
/// Desenhado cheio (não traço), como o ▶ do botão Abrir.
const ICONE_PLAY: &str = r#"<path d="M7 4v16l13-8z"/>"#;
const ICONE_PAUSA: &str = r#"<path d="M6 5h4v14H6zM14 5h4v14h-4z"/>"#;
const ICONE_X: &str = r#"<path d="M18 6L6 18M6 6l12 12"/>"#;
const ICONE_LIXO: &str = r#"<path d="M4 7h16M10 11v6M14 11v6M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2l1-12M9 7V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"/>"#;
const ICONE_PASTA: &str = r#"<path d="M5 19l2.757-7.351A1 1 0 0 1 8.693 11H21a1 1 0 0 1 .986 1.164l-.996 5.211A2 2 0 0 1 19.026 19z"/><path d="M5 19a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h4l3 3h7a2 2 0 0 1 2 2v2"/>"#;
const ICONE_MINIMIZAR: &str = r#"<path d="M5 12h14"/>"#;
const ICONE_MAXIMIZAR: &str = r#"<rect x="5" y="5" width="14" height="14" rx="1"/>"#;

thread_local! {
    // Cada SVG é analisado uma vez; o mesmo `Svg` também reaproveita o cache de rasterização.
    static SVGS: RefCell<HashMap<String, Svg>> = RefCell::new(HashMap::new());
}

fn svg_cache(chave: &str, fonte: impl FnOnce() -> String) -> Svg {
    SVGS.with(|c| c.borrow_mut().entry(chave.to_string()).or_insert_with(|| Svg::from_bytes(fonte().as_bytes()).expect("SVG embutido válido")).clone())
}

fn traco(caminho: &'static str, tamanho: f32, cor: Color) -> SvgView {
    let s = svg_cache(caminho, || format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{caminho}</svg>"##));
    svg(s).width(tamanho).height(tamanho).color(cor)
}

fn cheio(caminho: &'static str, tamanho: f32, cor: Color) -> SvgView {
    let s = svg_cache(&format!("cheio{caminho}"), || format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000" stroke="#000" stroke-width="2" stroke-linejoin="round">{caminho}</svg>"##));
    svg(s).width(tamanho).height(tamanho).color(cor)
}

/// A pena da marca na cor do app, sem o quadrado de fundo.
fn pena_na_cor(cor: &str) -> String {
    let clara = hex(cor).mix(Color::WHITE, 0.4);
    let b = |v: f32| (v * 255.0).round() as u8;
    let mut p = include_str!("../assets/pena.svg").replace("#287846", cor).replace("#72aa86", &format!("#{:02x}{:02x}{:02x}", b(clara.r), b(clara.g), b(clara.b)));
    if let Some(i) = p.find("<rect id=\"fundo-arredondado\"") {
        let fim = p[i..].find("/>").map_or(i, |f| i + f + 2);
        p.replace_range(i..fim, "");
    }
    p
}

fn pena<'a>(cor: &str, tamanho: f32, apagada: bool) -> Element<'a, Msg> {
    let s = svg_cache(&format!("pena{cor}"), || pena_na_cor(cor));
    let v = svg(s).width(tamanho).height(tamanho);
    // Sem opacidade no SVG: apagada, a pena vai em cinza.
    if apagada {
        v.color(mudo().alpha(0.7)).into()
    } else {
        v.into()
    }
}

// ---------- Estilos ----------

/// Vidro sobre a foto: gradiente, fio de luz na borda, sombra e o desfoque do que está atrás.
fn vidro(alfa: f32, raio: f32) -> Style {
    let (topo, baixo, fio, sombra) = if escuro() {
        let base = cor_painel();
        (Color { r: base.r + 0.06, g: base.g + 0.06, b: base.b + 0.08, a: alfa }, base.alpha(alfa), Color::WHITE.alpha(0.12), 0.45)
    } else {
        // A foto já vem clareada pelo véu (veja `view`): o vidro só precisa de um branco leve.
        let a = alfa * 0.6;
        (Color::WHITE.alpha(a + 0.1), Color::rgb8(240, 243, 248).alpha(a), Color::WHITE.alpha(0.55), 0.12)
    };
    Style::default()
        .background(Background::Linear { angle: std::f32::consts::FRAC_PI_2, stops: vec![(0.0, topo), (1.0, baixo)] })
        .border(Border::rounded(raio).color(fio, 1.0))
        .shadow(Shadow { color: Color::BLACK.alpha(sombra), offset: Point::new(0.0, 8.0), blur: 28.0 })
        .text_color(texto())
        .backdrop_blur(24.0)
}

/// A lista: quase opaca no escuro (o fundo atravessando as linhas atrapalhava a leitura); no
/// claro, vidro de verdade, com o desfoque segurando a leitura.
fn cartao(_: &Theme) -> Style {
    if escuro() {
        vidro(0.94, 12.0).backdrop_blur(0.0)
    } else {
        vidro(0.94, 12.0)
    }
}

fn destaque(_: &Theme, s: Status) -> Style {
    let a = hex(COR_LAUNCHER);
    let (topo, baixo) = match s {
        Status::Hovered => (a.mix(Color::WHITE, 0.12), a),
        Status::Pressed => (a.mix(Color::BLACK, 0.15), a.mix(Color::BLACK, 0.15)),
        Status::Disabled => (a.alpha(0.4), a.alpha(0.4)),
        _ => (a, a.mix(Color::BLACK, 0.14)),
    };
    Style::default()
        .background(Background::Linear { angle: std::f32::consts::FRAC_PI_2, stops: vec![(0.0, topo), (1.0, baixo)] })
        .border(Border::rounded(8.0).color(Color::WHITE.alpha(0.25), 1.0))
        .shadow(Shadow { color: a.alpha(if s == Status::Hovered { 0.55 } else { 0.35 }), offset: Point::new(0.0, 2.0), blur: 14.0 })
        .text_color(tinta_destaque())
}

fn perigo(_: &Theme, s: Status) -> Style {
    let fundo = if matches!(s, Status::Hovered | Status::Pressed) { hex("#a50e0e") } else { hex("#c5221f") };
    Style::default().background(fundo).border(Border::rounded(6.0)).text_color(Color::WHITE)
}

fn botao_vidro(_: &Theme, s: Status) -> Style {
    let fundo = match s {
        Status::Hovered => 0.12,
        Status::Pressed => 0.16,
        Status::Disabled => 0.02,
        _ => 0.05,
    };
    let tinta = if escuro() { Color::WHITE } else { Color::BLACK };
    Style::default().background(tinta.alpha(fundo)).border(Border::rounded(8.0).color(tinta.alpha(0.14), 1.0)).text_color(if s == Status::Disabled { mudo() } else { texto() })
}

fn pilula(ativo: bool) -> impl Fn(&Theme, Status) -> Style {
    move |_, s| {
        let sobre = matches!(s, Status::Hovered | Status::Pressed);
        let bg = if ativo { acento().alpha(0.16) } else { painel().alpha(if sobre { 0.75 } else { 0.5 }) };
        Style::default().background(bg).text_color(texto()).border(Border::rounded(10.0).color(if ativo { acento() } else { borda() }, if ativo { 1.5 } else { 1.0 }))
    }
}

/// Botões sem fundo até o hover: os da janela, a engrenagem, os itens do menu.
fn controle(fechar: bool) -> impl Fn(&Theme, Status) -> Style {
    move |_, s| {
        let sobre = matches!(s, Status::Hovered | Status::Pressed);
        let bg = sobre.then(|| if fechar { Color::rgb8(0xe8, 0x11, 0x23) } else { texto().alpha(0.1) });
        Style { background: bg.map(Into::into), border: Border::rounded(8.0), text_color: Some(texto()), ..Default::default() }
    }
}

fn caixa(cor: Color, raio: f32, contorno: Option<Color>) -> impl Fn(&Theme) -> Style {
    move |_| {
        let b = Border::rounded(raio);
        Style::default().background(cor).border(match contorno {
            Some(c) => b.color(c, 1.0),
            None => b,
        })
    }
}

fn ponto<'a>(cor: Color) -> Element<'a, Msg> {
    container(Space::new(8.0, 8.0)).style(caixa(cor, 4.0, None)).into()
}

fn linha_fina<'a>(cor: Color) -> Element<'a, Msg> {
    container(Space::new(Length::FILL, 1.0)).width(Length::FILL).style(caixa(cor, 0.0, None)).into()
}

// ---------- O app ----------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Filtro {
    Todos,
    Instalados,
    Atualizacoes,
    NaoInstalados,
}

#[derive(Debug, Clone)]
enum Msg {
    Verificar,
    Release(usize, Result<Option<Release>, String>),
    Instalar(usize),
    AtualizarTudo,
    Evento(usize, Evento),
    Pausar(usize),
    Cancelar(usize),
    Abrir(usize),
    Filtrar(Filtro),
    Buscar(String),
    Menu(usize),
    FecharMenu,
    MostrarPasta(usize),
    Novidades(usize),
    PedirDesinstalar(usize),
    Desinstalar(usize),
    Desinstalado(usize, Result<(), String>),
    Esc,
    Configuracoes,
    Tema(Tema),
    Grade(bool),
    IniciarComSistema(bool),
    VersaoExterna(usize, Option<String>),
    Amostrar,
    LauncherRelease(Result<Option<Release>, String>),
    AtualizarLauncher,
    LauncherAtualizado(Result<std::path::PathBuf, String>),
    Capturar,
    Tamanho(Size),
    Puxar(ResizeDirection),
    Arrastar,
    Minimizar,
    Maximizar,
    Fechar,
}

/// A versão nova do próprio launcher.
enum Autoatualizacao {
    Nada,
    Disponivel(Release),
    Atualizando,
    Falhou(String),
}

struct Launcher {
    linhas: Vec<Linha>,
    filtro: Filtro,
    busca: String,
    /// O menu "⋯" aberto e onde o clique aconteceu (o menu abre ali).
    menu: Option<(usize, Point)>,
    /// Onde o mouse está: o `on_event` só lê o evento, então guarda aqui sem gerar mensagem.
    cursor: Cell<Point>,
    confirmar: Option<usize>,
    configuracoes: bool,
    /// Primeira abertura: pergunta se o launcher deve abrir junto com o computador.
    perguntar_inicio: bool,
    inicia_com_sistema: bool,
    erro_inicio: Option<String>,
    tema: Tema,
    grade: bool,
    tamanho: Size,
    /// Tentou fechar no meio de uma instalação: o rodapé avisa.
    fechar_bloqueado: bool,
    launcher: Autoatualizacao,
    fundo: Image,
    /// "RAM 84 MB · CPU 0,3%"; a CPU é da máquina inteira, como no monitor do sistema.
    recursos: String,
    cpu_antes: Option<(u64, u64)>,
}

impl Launcher {
    fn novo(tema: Tema, grade: bool, perguntou_inicio: bool) -> (Self, Task<Msg>) {
        let img = ::image::load_from_memory(include_bytes!("../assets/fundo.jpg")).map(|i| i.to_rgba8()).unwrap_or_default();
        let (w, h) = img.dimensions();
        let mut l = Launcher {
            linhas: Vec::new(),
            filtro: Filtro::Todos,
            busca: String::new(),
            menu: None,
            cursor: Cell::new(Point::ORIGIN),
            confirmar: None,
            configuracoes: false,
            // A captura de tela não espera resposta de ninguém; no macOS ainda não há como abrir junto.
            perguntar_inicio: !perguntou_inicio && !cfg!(target_os = "macos") && std::env::var_os("FEATHERLAUNCHER_CAPTURAR").is_none(),
            inicia_com_sistema: instalar::inicia_com_sistema(),
            erro_inicio: None,
            tema,
            grade,
            tamanho: TAMANHO_INICIAL,
            fechar_bloqueado: false,
            launcher: Autoatualizacao::Nada,
            fundo: Image::from_rgba(w.max(1), h.max(1), if w == 0 { vec![0; 4] } else { img.into_raw() }).expect("tamanho confere"),
            recursos: String::new(),
            cpu_antes: None,
        };
        let mut tarefas = vec![
            l.verificar(),
            Task::perform(no_tokio(releases::ultima("featherlauncher")), Msg::LauncherRelease),
            Task::done(Msg::Amostrar),
            Task::run(tarefas::relogio(std::time::Duration::from_secs(1)), |_| Msg::Amostrar),
        ];
        // FEATHERLAUNCHER_CAPTURAR=tela.png fotografa a janela depois que as releases chegam e fecha.
        if std::env::var_os("FEATHERLAUNCHER_CAPTURAR").is_some() {
            tarefas.push(Task::perform(no_tokio(async { tokio::time::sleep(std::time::Duration::from_secs(4)).await }), |_| Msg::Capturar));
        }
        (l, Task::batch(tarefas))
    }

    fn verificar(&mut self) -> Task<Msg> {
        let registro = instalar::instalados();
        let mut antigas = std::mem::take(&mut self.linhas).into_iter();
        self.linhas = APPS
            .iter()
            .map(|app| Linha {
                consulta: Consulta::Verificando,
                // Instalado por fora: "?" até a versão chegar do `dpkg`/`rpm` (ver abaixo).
                instalada: registro.get(app.id).cloned().or_else(|| instalar::executavel(app).map(|_| "?".into())),
                // Download em andamento sobrevive ao "Verificar".
                download: antigas.next().and_then(|a| a.download),
                erro: None,
            })
            .collect();
        let externas = APPS.iter().enumerate().filter(|&(_, app)| !registro.contains_key(app.id)).map(|(i, app)| {
            let versao = no_tokio(async move { tokio::task::spawn_blocking(move || instalar::executavel(app).and_then(|exe| instalar::versao_externa(app, &exe))).await.ok().flatten() });
            Task::perform(versao, move |v| Msg::VersaoExterna(i, v))
        });
        Task::batch(APPS.iter().enumerate().map(|(i, app)| Task::perform(no_tokio(releases::ultima(app.id)), move |r| Msg::Release(i, r))).chain(externas))
    }

    fn instalar(&mut self, i: usize) -> Task<Msg> {
        let linha = &mut self.linhas[i];
        let Some(rel) = linha.release().cloned() else { return Task::none() };
        if matches!(linha.download, Some(Download::Baixando { .. } | Download::Instalando | Download::Desinstalando)) {
            return Task::none();
        }
        let (baixado, total) = match linha.download {
            Some(Download::Pausado { baixado, total }) => (baixado, total),
            _ => (0, rel.pacote.as_ref().map_or(0, |p| p.tamanho)),
        };
        let (tarefa, alca) = Task::run(instalar::baixar_e_instalar(&APPS[i], rel), move |e| Msg::Evento(i, e)).abortable();
        linha.download = Some(Download::Baixando { baixado, total, alca });
        linha.erro = None;
        tarefa
    }

    fn passa(&self, i: usize, f: Filtro) -> bool {
        let l = &self.linhas[i];
        match f {
            Filtro::Todos => true,
            Filtro::Instalados => l.instalada.is_some(),
            Filtro::Atualizacoes => l.atualizacao().is_some(),
            Filtro::NaoInstalados => l.instalada.is_none(),
        }
    }

    fn contar(&self, f: Filtro) -> usize {
        (0..self.linhas.len()).filter(|&i| self.passa(i, f)).count()
    }

    fn ocupado(&self) -> bool {
        self.linhas.iter().any(|l| matches!(l.download, Some(Download::Instalando | Download::Desinstalando)))
    }

    // ---------- Partes da tela ----------

    fn barra(&self) -> Element<'_, Msg> {
        let nome = row![
            pena(COR_LAUNCHER, 28.0, false),
            text("FeatherWorks").size(16).font(Font::SEMIBOLD),
            text("Launcher").size(15).color(mudo()),
            horizontal_space(),
            self.botao_launcher(),
            tooltip(button(traco(ICONE_CONFIG, 18.0, texto())).on_press(Msg::Configuracoes).padding(7.0).style(controle(false)), "Configurações", TooltipPosition::Bottom),
        ]
        .spacing(10)
        .align(Align::Center)
        .padding([0u16, 14])
        .width(Length::FILL)
        .height(50.0);
        let janela = |icone, m, fechar| button(container(traco(icone, 15.0, mudo())).center()).on_press(m).width(40.0).height(32.0).padding(0.0).style(controle(fechar));
        row![
            mouse_area(nome).on_press(Msg::Arrastar).on_double_click(Msg::Maximizar),
            row![janela(ICONE_MINIMIZAR, Msg::Minimizar, false), janela(ICONE_MAXIMIZAR, Msg::Maximizar, false), janela(ICONE_X, Msg::Fechar, true)].spacing(4).padding(Padding { right: 8.0, ..Padding::ZERO }),
        ]
        .align(Align::Center)
        .into()
    }

    /// "Atualizar launcher para X" na barra de título, só quando há versão nova.
    fn botao_launcher(&self) -> Element<'_, Msg> {
        let botao = |rotulo: String, m: Option<Msg>| button(text(rotulo).size(12).font(Font::SEMIBOLD)).on_press_maybe(m).padding([4u16, 12]).style(destaque).into();
        match &self.launcher {
            Autoatualizacao::Nada => Space::new(0.0, 0.0).into(),
            Autoatualizacao::Disponivel(rel) => botao(format!("Atualizar launcher para {}", rel.versao), Some(Msg::AtualizarLauncher)),
            Autoatualizacao::Atualizando => botao("Atualizando o launcher…".into(), None),
            Autoatualizacao::Falhou(e) => text(format!("Launcher não atualizou: {e}")).size(12).color(erro()).into(),
        }
    }

    /// Filtros em pílulas com a contagem, alternar lista/grade, busca e Verificar.
    fn faixa(&self) -> Element<'_, Msg> {
        let aba = |rotulo: &'static str, f: Filtro| {
            let n = self.contar(f);
            let alerta = f == Filtro::Atualizacoes && n > 0;
            let contagem = container(text(n.to_string()).size(11).font(Font::BOLD).color(if alerta { Color::WHITE } else { texto() })).padding([1u16, 7]).style(caixa(if alerta { hex(LARANJA) } else { texto().alpha(0.12) }, 9.0, None));
            button(row![text(rotulo).size(13), contagem].spacing(8).align(Align::Center)).on_press(Msg::Filtrar(f)).padding([7u16, 14]).style(pilula(self.filtro == f))
        };
        let modo = |icone, grade: bool, dica: &'static str| tooltip(button(traco(icone, 16.0, texto())).on_press(Msg::Grade(grade)).padding([6u16, 8]).style(pilula(self.grade == grade)), dica, TooltipPosition::Bottom);
        let busca = container(
            row![traco(ICONE_BUSCA, 15.0, mudo()), text_input("Buscar aplicativo…", &self.busca).on_input(Msg::Buscar).size(13).padding(0.0).style(|_, _| Style::default().text_color(texto()))].spacing(8).align(Align::Center),
        )
        .padding([0u16, 12])
        .width(240.0)
        .height(34.0)
        .align_y(Align::Center)
        .style(caixa(painel().alpha(0.6), 8.0, Some(borda())));
        let atualizaveis = self.contar(Filtro::Atualizacoes);
        row![
            aba("Todos", Filtro::Todos),
            aba("Instalados", Filtro::Instalados),
            aba("Atualizações", Filtro::Atualizacoes),
            aba("Não instalados", Filtro::NaoInstalados),
            horizontal_space(),
            row![modo(ICONE_LISTA, false, "Lista"), modo(ICONE_GRADE, true, "Grade")].spacing(2).padding(Padding { right: 8.0, ..Padding::ZERO }),
            busca,
            button(row![traco(ICONE_REFRESH, 15.0, texto()), text("Verificar").size(13).font(Font::SEMIBOLD)].spacing(6).align(Align::Center)).on_press(Msg::Verificar).padding([8u16, 14]).style(pilula(false)),
        ]
        .push_maybe((atualizaveis > 0).then(|| button(text(format!("Atualizar tudo ({atualizaveis})")).size(13).font(Font::SEMIBOLD)).on_press(Msg::AtualizarTudo).padding([8u16, 14]).style(destaque)))
        .spacing(8)
        .align(Align::Center)
        .padding(Padding { top: 4.0, right: 16.0, bottom: 12.0, left: 16.0 })
        .width(Length::FILL)
        .into()
    }

    fn conteudo(&self) -> Element<'_, Msg> {
        let busca = self.busca.to_lowercase();
        let visiveis: Vec<usize> = (0..APPS.len())
            .filter(|&i| {
                let app = &APPS[i];
                self.passa(i, self.filtro) && (app.nome().to_lowercase().contains(&busca) || app.descricao.to_lowercase().contains(&busca) || app.etiquetas.iter().any(|e| e.to_lowercase().contains(&busca)))
            })
            .collect();
        let vazio = || container(text("Nenhum aplicativo neste filtro.").size(13).color(mudo())).padding(22.0);
        let corpo: Element<'_, Msg> = if self.grade {
            if visiveis.is_empty() {
                vazio().style(cartao).width(Length::FILL).into()
            } else {
                visiveis.iter().fold(row![].spacing(16), |r, &i| r.push(self.cartao(i))).wrap().into()
            }
        } else if visiveis.is_empty() {
            container(vazio()).width(Length::FILL).style(cartao).into()
        } else {
            let cab = |s: &'static str| text(s).size(11).font(Font::BOLD).color(mudo());
            let cabecalho = row![
                cab("APLICATIVO").width(Length::FILL),
                cab("VERSÃO").width(COL_VERSAO),
                cab("ESTADO").width(COL_ESTADO),
                cab("ÚLTIMA ATUALIZAÇÃO").width(COL_DATA),
                container(cab("AÇÕES")).width(COL_ACOES).align_x(Align::End),
            ]
            .spacing(16)
            .padding([12u16, 20]);
            let lista = visiveis.iter().fold(column![cabecalho], |c, &i| c.push(linha_fina(borda_suave())).push(self.linha(i)));
            container(lista).width(Length::FILL).style(cartao).into()
        };
        scrollable(container(corpo).padding([8u16, 12])).width(Length::FILL).height(Length::FILL).into()
    }

    fn linha(&self, i: usize) -> Element<'_, Msg> {
        let (nome, col_versao) = self.partes(i);
        row![
            row![pena(APPS[i].cor, 48.0, self.linhas[i].em_breve()), nome].spacing(16).align(Align::Center).width(Length::FILL),
            col_versao.width(COL_VERSAO),
            container(self.estado(i)).width(COL_ESTADO),
            container(self.data(i)).width(COL_DATA),
            container(self.acoes(i)).width(COL_ACOES).align_x(Align::End),
        ]
        .spacing(16)
        .padding([14u16, 20])
        .align(Align::Center)
        .into()
    }

    /// O app no modo grade: o mesmo conteúdo da linha, empilhado num cartão.
    fn cartao(&self, i: usize) -> Element<'_, Msg> {
        let (nome, col_versao) = self.partes(i);
        container(
            column![
                row![pena(APPS[i].cor, 52.0, self.linhas[i].em_breve()), nome.width(Length::FILL)].spacing(14).align(Align::Center),
                row![col_versao, horizontal_space(), self.estado(i)].align(Align::Center),
                row![self.data(i), horizontal_space()].align(Align::Center),
                container(self.acoes(i)).width(Length::FILL).height(36.0).align_x(Align::End).align_y(Align::Center),
            ]
            .spacing(14),
        )
        .padding(20.0)
        .width(360.0)
        .style(cartao)
        .into()
    }

    /// Nome com descrição, etiquetas e avisos, e a coluna da versão: comuns à linha e ao cartão.
    fn partes(&self, i: usize) -> (Flex<'_, Msg>, Flex<'_, Msg>) {
        let app = &APPS[i];
        let l = &self.linhas[i];
        let rel = l.release();
        let apagado = l.em_breve();

        // Cada etiqueta entra solta na linha, para quebrar inteira.
        let descricao = etiquetas(app).into_iter().fold(row![text(app.descricao).size(13).color(mudo())], |r, e| r.push(e)).spacing(8).align(Align::Center).wrap();
        let mut nome = column![row![text("Feather").size(17).font(Font::BOLD).color(if apagado { mudo() } else { texto() }), text(app.sufixo).size(17).font(Font::BOLD).color(tom_app(app.cor))], descricao].spacing(5);
        let falha = match &l.consulta {
            // O limite do GitHub sai uma vez só, no rodapé.
            Consulta::Falhou(e) if !e.starts_with(releases::LIMITE) => Some(e),
            _ => None,
        };
        if let Some(e) = l.erro.as_ref().or(falha) {
            nome = nome.push(text(e.as_str()).size(12).color(erro()));
        }
        if self.confirmar == Some(i) {
            let aviso = if instalar::pelo_sistema(app) { format!("Desinstalar {}? É pacote do sistema: ele vai pedir sua senha.", app.nome()) } else { format!("Desinstalar {}?", app.nome()) };
            nome = nome.push(text(aviso).size(12).color(erro()));
        }

        let v = |s: String| text(s).size(14);
        let versao: Element<'_, Msg> = match (l.atualizacao(), l.instalada.as_deref(), rel) {
            (Some(r), Some(inst), _) => row![v(inst.to_string()).color(mudo()), v("→".into()).color(mudo()), v(r.versao.clone()).color(tom(LARANJA))].spacing(6).into(),
            (None, Some(inst), _) if inst != "?" => v(inst.to_string()).into(),
            (_, _, Some(r)) => v(r.versao.clone()).into(),
            _ => v("—".into()).color(mudo()).into(),
        };
        let col_versao = column![versao].spacing(4);
        let col_versao = match rel {
            Some(_) => col_versao.push(button(text("Ver novidades ›").size(12).color(tom(AZUL))).on_press(Msg::Novidades(i)).padding(0.0).style(|_, _| Style::default())),
            None if apagado => col_versao.push(text("Em desenvolvimento").size(12).color(mudo())),
            None => col_versao,
        };
        (nome, col_versao)
    }

    /// Data da última release, ou por que não há.
    fn data(&self, i: usize) -> Element<'_, Msg> {
        match self.linhas[i].release() {
            Some(r) => text(r.data.as_str()).size(13).into(),
            None if self.linhas[i].em_breve() => text("Ainda sem versão publicada").size(12).color(mudo()).into(),
            None => text("—").size(13).color(mudo()).into(),
        }
    }

    fn estado(&self, i: usize) -> Element<'_, Msg> {
        let l = &self.linhas[i];
        let (rotulo, cor) = match (&l.download, &l.consulta) {
            (Some(Download::Baixando { .. }), _) => ("Baixando…".to_string(), tom(AZUL)),
            (Some(Download::Pausado { .. }), _) => ("Pausado".into(), tom(LARANJA)),
            (Some(Download::Instalando), _) => ("Instalando…".into(), tom(AZUL)),
            (Some(Download::Desinstalando), _) => ("Desinstalando…".into(), tom(AZUL)),
            (None, Consulta::Verificando) if l.instalada.is_none() => ("Verificando…".into(), mudo()),
            (None, Consulta::Falhou(e)) if l.instalada.is_none() && e.starts_with(releases::LIMITE) => ("Aguardando o GitHub".into(), mudo()),
            (None, Consulta::Falhou(_)) if l.instalada.is_none() => ("Sem conexão".into(), erro()),
            _ if l.atualizacao().is_some() => ("Atualização disponível".into(), tom(LARANJA)),
            _ if l.instalada.is_some() => ("Instalado".into(), tom(VERDE)),
            _ if l.em_breve() => {
                return container(row![traco(ICONE_RELOGIO, 15.0, texto()), text("Em breve").size(13).font(Font::SEMIBOLD)].spacing(8).align(Align::Center)).padding([6u16, 14]).style(caixa(texto().alpha(0.06), 16.0, Some(borda()))).into();
            }
            _ if l.instalavel().is_some() => ("Não instalado".into(), mudo()),
            _ => (format!("Sem pacote para {}", releases::nome_do_sistema()), mudo()),
        };
        container(row![ponto(cor), text(rotulo).size(13).font(Font::SEMIBOLD).color(cor)].spacing(8).align(Align::Center)).padding([6u16, 14]).style(caixa(cor.alpha(0.18), 16.0, Some(cor.alpha(0.35)))).into()
    }

    fn acoes(&self, i: usize) -> Element<'_, Msg> {
        let l = &self.linhas[i];
        // Todo botão de ação tem o mesmo tamanho; o "⋯" e os de download são quadrados de 36.
        let primario = |d: Element<'static, Msg>, m: Msg, w: f32| button(container(d).center()).on_press(m).width(w).height(36.0).padding(0.0).style(destaque);
        let quadrado = |d: Element<'static, Msg>, m: Msg| button(container(d).center()).on_press(m).width(36.0).height(36.0).padding(0.0).style(botao_vidro);
        let barra = |baixado: u64, total: u64| {
            let pct = if total > 0 { baixado as f32 * 100.0 / total as f32 } else { 0.0 };
            column![text(format!("{pct:.0}% de {}", megas(total))).size(12).color(mudo()), progress_bar(0.0..=100.0, pct).height(6.0).color(hex(VERDE))].spacing(6).width(Length::FILL)
        };
        let aviso = |s: &'static str| row![text(s).size(12).color(mudo())];
        let linha = match &l.download {
            Some(Download::Baixando { baixado, total, .. }) => row![barra(*baixado, *total), quadrado(cheio(ICONE_PAUSA, 14.0, texto()).into(), Msg::Pausar(i)), quadrado(traco(ICONE_X, 16.0, texto()).into(), Msg::Cancelar(i))],
            Some(Download::Pausado { baixado, total }) => row![barra(*baixado, *total), quadrado(cheio(ICONE_PLAY, 14.0, texto()).into(), Msg::Instalar(i)), quadrado(traco(ICONE_X, 16.0, texto()).into(), Msg::Cancelar(i))],
            Some(Download::Instalando) => aviso("Conferindo o SHA256 e instalando…"),
            Some(Download::Desinstalando) => aviso("Removendo do computador…"),
            None if self.confirmar == Some(i) => row![
                button(text("Cancelar").size(13)).on_press(Msg::Esc).padding([9u16, 14]).style(botao_vidro),
                button(text("Desinstalar").size(13).font(Font::SEMIBOLD)).on_press(Msg::Desinstalar(i)).padding([9u16, 16]).style(perigo),
            ],
            None => {
                let mais = || quadrado(traco(ICONE_MAIS, 16.0, texto()).into(), Msg::Menu(i));
                // Sem "⋯" (nada a fazer no menu), um vão do mesmo tamanho mantém as colunas.
                let vao = || Space::new(36.0, 36.0);
                match (l.atualizacao(), l.instalada.is_some(), l.instalavel()) {
                    // Com atualização o "Abrir" encolhe para o ícone; a versão nova está na coluna Versão.
                    (Some(_), _, _) => row![quadrado(cheio(ICONE_PLAY, 14.0, texto()).into(), Msg::Abrir(i)), primario(text("Atualizar").size(13).font(Font::SEMIBOLD).into(), Msg::Instalar(i), LARGURA_BOTAO - 44.0), mais()],
                    (None, true, _) => row![primario(row![cheio(ICONE_PLAY, 15.0, tinta_destaque()), text("Abrir").size(15).font(Font::SEMIBOLD)].spacing(8).align(Align::Center).into(), Msg::Abrir(i), LARGURA_BOTAO), mais()],
                    (None, false, Some(r)) => {
                        let tamanho = r.pacote.as_ref().filter(|p| p.tamanho > 0).map_or(String::new(), |p| format!(" · {}", megas(p.tamanho)));
                        let rotulo = row![traco(ICONE_BAIXAR, 16.0, tinta_destaque()), text(format!("Instalar{tamanho}")).size(14).font(Font::SEMIBOLD)].spacing(8).align(Align::Center);
                        row![primario(rotulo.into(), Msg::Instalar(i), LARGURA_BOTAO), vao()]
                    }
                    _ if l.em_breve() => row![
                        button(container(row![traco(ICONE_CADEADO, 15.0, mudo()), text("Indisponível").size(13).color(mudo())].spacing(8).align(Align::Center)).center()).width(LARGURA_BOTAO).height(36.0).padding(0.0).style(botao_vidro),
                        vao(),
                    ],
                    _ => row![],
                }
            }
        };
        linha.spacing(8).align(Align::Center).into()
    }

    /// O menu do "⋯", aberto onde foi o clique; clicar fora fecha.
    fn menu_mais(&self, i: usize, onde: Point) -> Element<'_, Msg> {
        let item = |icone: Element<'static, Msg>, rotulo: String, m: Msg, perigoso: bool| {
            button(row![icone, text(rotulo).size(13).color(if perigoso { erro() } else { texto() })].spacing(10).align(Align::Center)).on_press(m).width(Length::FILL).padding([8u16, 10]).style(controle(false))
        };
        let painel = container(
            column![
                item(traco(ICONE_PASTA, 16.0, texto()).into(), "Mostrar pasta de instalação".into(), Msg::MostrarPasta(i), false),
                container(linha_fina(borda_suave())).padding([4u16, 0]),
                item(traco(ICONE_LIXO, 16.0, erro()).into(), format!("Desinstalar {}…", APPS[i].nome()), Msg::PedirDesinstalar(i), true),
            ]
            .spacing(2),
        )
        .padding(6.0)
        .width(LARGURA_MENU)
        .style(|_| Style::default().background(painel()).border(Border::rounded(10.0).color(borda(), 1.0)).shadow(Shadow { color: Color::BLACK.alpha(0.35), offset: Point::new(0.0, 6.0), blur: 20.0 }));
        // Abre para baixo e para a esquerda do clique; perto da borda de baixo, para cima.
        let x = (onde.x - LARGURA_MENU + 18.0).clamp(8.0, (self.tamanho.width - LARGURA_MENU - 8.0).max(8.0));
        let y = if onde.y + 110.0 > self.tamanho.height { onde.y - 110.0 } else { onde.y + 18.0 };
        let posto = container(opaque(painel)).padding(Padding { top: y.max(0.0), left: x, right: 0.0, bottom: 0.0 });
        mouse_area(container(posto).width(Length::FILL).height(Length::FILL)).on_press(Msg::FecharMenu).into()
    }

    fn configuracoes(&self) -> Element<'_, Msg> {
        let secao = |s: &'static str| text(s).size(12).font(Font::BOLD).color(mudo());
        let valor = |rotulo: &'static str, v: String| row![text(rotulo).size(13).color(mudo()).width(120.0), text(v).size(13)].align(Align::Center);
        let opcoes = Tema::TODOS.iter().fold(row![].spacing(4), |r, &(t, _, rotulo)| r.push(button(text(rotulo).size(13)).on_press(Msg::Tema(t)).padding([6u16, 14]).style(pilula(self.tema == t))));
        let topo = row![traco(ICONE_CONFIG, 18.0, texto()), text("Configurações").size(15).font(Font::SEMIBOLD), horizontal_space(), button(traco(ICONE_X, 15.0, mudo())).on_press(Msg::Esc).padding(6.0).style(controle(false))].spacing(10).align(Align::Center);
        let dialogo = container(
            column![
                topo,
                secao("APARÊNCIA"),
                row![text("Tema").size(13).color(mudo()).width(120.0), opcoes].align(Align::Center),
                secao("INSTALAÇÃO"),
                valor("Pasta", instalar::destino().into()),
                valor("Versões de", format!("github.com/{}", releases::ORG)),
                valor("Launcher", env!("CARGO_PKG_VERSION").into()),
                toggler("Ícone na bandeja e aviso de atualizações ao ligar o computador", self.inicia_com_sistema).size(13).on_toggle(Msg::IniciarComSistema),
            ]
            .spacing(12),
        );
        modal(dialogo)
    }

    fn pergunta_inicio(&self) -> Element<'_, Msg> {
        let dialogo = container(
            column![
                row![pena(COR_LAUNCHER, 28.0, false), text("Avisar das atualizações?").size(16).font(Font::SEMIBOLD)].spacing(10).align(Align::Center),
                text("Quando você entrar no computador, o launcher fica como um ícone pequeno na bandeja, confere se algum aplicativo tem versão nova e avisa por notificação. Dá para mudar depois em Configurações.").size(13).color(mudo()),
                row![
                    horizontal_space(),
                    button(text("Agora não").size(13)).on_press(Msg::IniciarComSistema(false)).padding([9u16, 14]).style(botao_vidro),
                    button(text("Sim, avisar").size(13).font(Font::SEMIBOLD)).on_press(Msg::IniciarComSistema(true)).padding([9u16, 16]).style(destaque),
                ]
                .spacing(8),
            ]
            .spacing(14),
        );
        modal(dialogo)
    }

    /// O que dizer no rodapé: o fechar bloqueado, a cota do GitHub estourada ou "Verificando…".
    fn situacao(&self) -> Option<(String, Color)> {
        if let Some(e) = &self.erro_inicio {
            return Some((format!("Não deu para ligar o aviso de atualizações: {e}"), tom(LARANJA)));
        }
        if self.fechar_bloqueado && self.ocupado() {
            return Some(("Espere terminar a instalação para fechar".into(), tom(LARANJA)));
        }
        let limite = self.linhas.iter().find_map(|l| match &l.consulta {
            Consulta::Falhou(e) if e.starts_with(releases::LIMITE) => Some(e.clone()),
            _ => None,
        });
        match limite {
            Some(e) => Some((format!("Consulta pausada: {e}"), tom(LARANJA))),
            None if self.linhas.iter().any(|l| matches!(l.consulta, Consulta::Verificando)) => Some(("Verificando…".into(), mudo())),
            None => None,
        }
    }

    fn rodape(&self) -> Element<'_, Msg> {
        let m = |s: String| text(s).size(12).color(mudo());
        let sep = || container(Space::new(1.0, 14.0)).style(caixa(borda(), 0.0, None));
        let situacao: Element<'_, Msg> = match self.situacao() {
            Some((s, cor)) => text(s).size(12).color(cor).into(),
            None => row![traco(ICONE_PACOTE, 14.0, mudo()), m("Cada pacote é conferido pelo SHA256 da release".into())].spacing(8).align(Align::Center).into(),
        };
        container(
            row![
                row![ponto(hex(VERDE)), m(format!("Instalando para {}", releases::nome_do_sistema())), m(instalar::destino().into())].spacing(8).align(Align::Center),
                sep(),
                situacao,
                horizontal_space(),
                text(self.recursos.as_str()).size(12).color(mudo()),
                sep(),
                row![traco(ICONE_GITHUB, 14.0, mudo()), m(format!("github.com/{}", releases::ORG))].spacing(8).align(Align::Center),
                sep(),
                m(format!("Launcher {}", env!("CARGO_PKG_VERSION"))),
            ]
            .spacing(14)
            .align(Align::Center),
        )
        .padding([0u16, 20])
        .height(ALTURA_RODAPE)
        .width(Length::FILL)
        .align_y(Align::Center)
        .style(|_| vidro(0.6, RAIO_VIDRO))
        .into()
    }

    /// Bordas da direita, de baixo e o canto: puxar redimensiona (sem moldura do sistema).
    fn alcas(&self) -> Element<'_, Msg> {
        let alca = |w: Length, h: Length, d: ResizeDirection, i: CursorIcon| mouse_area(Space::new(w, h)).on_press(Msg::Puxar(d)).cursor_icon(i);
        stack![
            container(alca(Length::Fixed(6.0), Length::FILL, ResizeDirection::East, CursorIcon::EResize)).width(Length::FILL).height(Length::FILL).align_x(Align::End),
            container(alca(Length::FILL, Length::Fixed(6.0), ResizeDirection::South, CursorIcon::SResize)).width(Length::FILL).height(Length::FILL).align_y(Align::End),
            container(alca(Length::Fixed(16.0), Length::Fixed(16.0), ResizeDirection::SouthEast, CursorIcon::SeResize)).width(Length::FILL).height(Length::FILL).align_x(Align::End).align_y(Align::End),
        ]
        .into()
    }
}

impl App for Launcher {
    type Msg = Msg;

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Verificar => return self.verificar(),
            Msg::Release(i, r) => {
                self.linhas[i].consulta = match r {
                    Ok(rel) => Consulta::Pronta(rel),
                    Err(e) => Consulta::Falhou(e),
                }
            }
            Msg::Instalar(i) => return self.instalar(i),
            Msg::AtualizarTudo => {
                let alvos: Vec<usize> = (0..self.linhas.len()).filter(|&i| self.linhas[i].atualizacao().is_some()).collect();
                return Task::batch(alvos.into_iter().map(|i| self.instalar(i)));
            }
            Msg::Evento(i, e) => {
                let linha = &mut self.linhas[i];
                match e {
                    Evento::Progresso { baixado: b, total: t } => {
                        if let Some(Download::Baixando { baixado, total, .. }) = &mut linha.download {
                            (*baixado, *total) = (b, t);
                        }
                    }
                    Evento::Instalando => linha.download = Some(Download::Instalando),
                    Evento::Fim(r) => {
                        linha.download = None;
                        match r {
                            Ok(v) => linha.instalada = Some(v),
                            Err(e) => linha.erro = Some(e),
                        }
                    }
                }
            }
            Msg::Pausar(i) => {
                let linha = &mut self.linhas[i];
                if let Some(Download::Baixando { baixado, total, alca }) = linha.download.take() {
                    alca.abort();
                    linha.download = Some(Download::Pausado { baixado, total });
                }
            }
            Msg::Cancelar(i) => {
                let linha = &mut self.linhas[i];
                if let Some(Download::Baixando { alca, .. }) = &linha.download {
                    alca.abort();
                }
                linha.download = None;
                if let Some((p, rel_versao)) = linha.release().and_then(|r| Some((r.pacote.as_ref()?, r.versao.clone()))) {
                    let _ = std::fs::remove_file(instalar::parcial(p, &rel_versao));
                }
            }
            Msg::Abrir(i) => self.linhas[i].erro = instalar::abrir(&APPS[i]).err(),
            Msg::Filtrar(f) => self.filtro = f,
            Msg::Buscar(s) => self.busca = s,
            Msg::Menu(i) => self.menu = if self.menu.is_some_and(|(m, _)| m == i) { None } else { Some((i, self.cursor.get())) },
            Msg::FecharMenu => self.menu = None,
            Msg::MostrarPasta(i) => {
                self.menu = None;
                self.linhas[i].erro = instalar::mostrar_pasta(&APPS[i]).err();
            }
            Msg::Novidades(i) => {
                if let Some(r) = self.linhas[i].release() {
                    let url = format!("https://github.com/{}/{}/releases/tag/v{}", releases::ORG, APPS[i].id, r.versao);
                    self.linhas[i].erro = instalar::abrir_externo(url.as_ref()).err();
                }
            }
            Msg::PedirDesinstalar(i) => {
                self.menu = None;
                self.confirmar = Some(i);
            }
            Msg::Desinstalar(i) => {
                self.confirmar = None;
                let linha = &mut self.linhas[i];
                if linha.download.is_some() {
                    return Task::none();
                }
                linha.download = Some(Download::Desinstalando);
                linha.erro = None;
                let feito = no_tokio(async move { tokio::task::spawn_blocking(move || instalar::desinstalar(&APPS[i])).await.map_err(|e| e.to_string())? });
                return Task::perform(feito, move |r| Msg::Desinstalado(i, r));
            }
            Msg::Desinstalado(i, r) => {
                let linha = &mut self.linhas[i];
                linha.download = None;
                match r {
                    Ok(()) => linha.instalada = None,
                    Err(e) => linha.erro = Some(e),
                }
            }
            Msg::Esc if self.perguntar_inicio => return self.update(Msg::IniciarComSistema(false)),
            Msg::Esc => {
                self.menu = None;
                self.confirmar = None;
                self.configuracoes = false;
            }
            Msg::Configuracoes => self.configuracoes = !self.configuracoes,
            Msg::Tema(t) => {
                self.tema = t;
                aplicar_tema(t);
                guardar_preferencias(t, self.grade);
            }
            Msg::IniciarComSistema(sim) => {
                if self.perguntar_inicio {
                    self.perguntar_inicio = false;
                    alterar_preferencias(|j| {
                        j.insert("perguntou_inicio".into(), true.into());
                    });
                }
                self.erro_inicio = instalar::iniciar_com_sistema(sim).err();
                self.inicia_com_sistema = instalar::inicia_com_sistema();
            }
            Msg::Grade(g) => {
                self.grade = g;
                guardar_preferencias(self.tema, g);
            }
            Msg::VersaoExterna(i, v) => {
                let linha = &mut self.linhas[i];
                if linha.instalada.as_deref() == Some("?") {
                    linha.instalada = v.or(linha.instalada.take());
                }
            }
            Msg::Amostrar => {
                let agora = consumo::tempos_de_cpu();
                let cpu = match (agora, self.cpu_antes) {
                    (Some((p, t)), Some((p0, t0))) => format!(" · CPU {:.1}%", (p.saturating_sub(p0) as f32 / t.saturating_sub(t0).max(1) as f32 * 100.0).clamp(0.0, 100.0)).replace('.', ","),
                    _ => String::new(),
                };
                self.cpu_antes = agora;
                self.recursos = format!("RAM {} MB{cpu}", consumo::rss_kb().unwrap_or(0).div_ceil(1024));
            }
            Msg::LauncherRelease(r) => {
                if let Ok(Some(rel)) = r {
                    if rel.pacote.is_some() && mais_nova(&rel.versao, env!("CARGO_PKG_VERSION")) {
                        self.launcher = Autoatualizacao::Disponivel(rel);
                    }
                }
            }
            Msg::AtualizarLauncher => {
                // Reabrir no meio de uma instalação a mataria.
                if self.linhas.iter().any(|l| l.download.is_some()) {
                    self.launcher = Autoatualizacao::Falhou("termine as instalações antes de atualizar o launcher".into());
                    return Task::none();
                }
                let rel = match std::mem::replace(&mut self.launcher, Autoatualizacao::Atualizando) {
                    Autoatualizacao::Disponivel(rel) => rel,
                    outro => {
                        self.launcher = outro;
                        return Task::none();
                    }
                };
                return Task::perform(no_tokio(instalar::atualizar_launcher(rel)), Msg::LauncherAtualizado);
            }
            Msg::LauncherAtualizado(r) => match r.and_then(|exe| std::process::Command::new(exe).spawn().map_err(|e| e.to_string())) {
                Ok(_) => std::process::exit(0),
                Err(e) => self.launcher = Autoatualizacao::Falhou(e),
            },
            Msg::Capturar => {
                let destino = std::env::var_os("FEATHERLAUNCHER_CAPTURAR").unwrap_or_default();
                let png = capture(&self.view(), self.tamanho, 1.0, &Theme { background: Color::TRANSPARENT, ..self.theme() }, &mut Renderer::new(), None);
                if let Err(e) = std::fs::write(&destino, png) {
                    eprintln!("[captura] {e}");
                }
                std::process::exit(0);
            }
            Msg::Tamanho(s) => self.tamanho = s,
            Msg::Puxar(d) => return Task::window(WindowCmd::DragResize(d)),
            Msg::Arrastar => return Task::window(WindowCmd::Drag),
            Msg::Minimizar => return Task::window(WindowCmd::Minimize),
            Msg::Maximizar => return Task::window(WindowCmd::ToggleMaximize),
            Msg::Fechar => {
                // Fechar no meio mata a cópia dos arquivos e deixa o app pela metade.
                if self.ocupado() {
                    self.fechar_bloqueado = true;
                    return Task::none();
                }
                return Task::window(WindowCmd::Close);
            }
        }
        Task::none()
    }

    fn theme(&self) -> Theme {
        let base = if escuro() { Theme::dark() } else { Theme::light() };
        Theme { background: par("#f1f3f4", "#181a1d"), surface: painel(), text: texto(), muted: mudo(), primary: acento(), border: borda(), danger: erro(), ..base }
    }

    fn on_event(&self, ev: &Event) -> Option<Msg> {
        match ev {
            Event::CursorMoved(p) => {
                self.cursor.set(*p);
                None
            }
            Event::Resized(s) => Some(Msg::Tamanho(*s)),
            Event::CloseRequested => Some(Msg::Fechar),
            Event::Key { key: Key::Named(NamedKey::Escape), pressed: true, .. } => Some(Msg::Esc),
            _ => None,
        }
    }

    fn view(&self) -> Element<'_, Msg> {
        // Cabeçalho e rodapé flutuam em vidro; a lista é um cartão quase opaco.
        let cabecalho = container(column![self.barra(), self.faixa()]).height(ALTURA_CABECALHO).width(Length::FILL).style(|_| vidro(0.6, RAIO_VIDRO));
        let corpo = column![cabecalho, self.conteudo(), self.rodape()].spacing(12).padding(MARGEM).width(Length::FILL).height(Length::FILL);
        let fundo = image(self.fundo.clone()).content_fit(ContentFit::Cover).width(Length::FILL).height(Length::FILL);
        // No claro, um véu branco deixa a foto em tons pastel: vidro claro sobre foto escura fica cinza.
        let veu = container(Space::new(Length::FILL, Length::FILL)).style(caixa(Color::WHITE.alpha(if escuro() { 0.0 } else { 0.42 }), 0.0, None));
        let mut camadas = stack![fundo, veu, corpo, self.alcas()];
        if self.perguntar_inicio {
            camadas = camadas.push(self.pergunta_inicio());
        } else if self.configuracoes {
            camadas = camadas.push(self.configuracoes());
        } else if let Some((i, onde)) = self.menu {
            camadas = camadas.push(self.menu_mais(i, onde));
        }
        // A foto e tudo por cima ficam dentro dos cantos da janela.
        container(camadas).width(Length::FILL).height(Length::FILL).clip(true).style(|_| Style::default().border(Border::rounded(RAIO_JANELA).color(borda(), 1.0)).text_color(texto())).into()
    }
}

/// Um diálogo no meio da janela; o véu escurece o resto e, clicado, fecha (`Msg::Esc`).
fn modal<'a>(dialogo: Container<'a, Msg>) -> Element<'a, Msg> {
    let dialogo = dialogo
        .padding(20.0)
        .width(460.0)
        .style(|_| Style::default().background(painel()).border(Border::rounded(12.0).color(borda(), 1.0)).shadow(Shadow { color: Color::BLACK.alpha(0.4), offset: Point::new(0.0, 10.0), blur: 32.0 }).text_color(texto()));
    let veu = mouse_area(container(Space::new(Length::FILL, Length::FILL)).width(Length::FILL).height(Length::FILL).style(caixa(Color::BLACK.alpha(if escuro() { 0.6 } else { 0.45 }), RAIO_JANELA, None))).on_press(Msg::Esc);
    stack![veu, container(opaque(dialogo)).width(Length::FILL).height(Length::FILL).center()].into()
}

/// Até quatro etiquetas; com mais, três e um "+N".
fn etiquetas<'a>(app: &AppInfo) -> Vec<Element<'a, Msg>> {
    let chip = |s: String| -> Element<'a, Msg> { container(text(s).size(11).color(texto())).padding([3u16, 9]).style(caixa(texto().alpha(0.06), 6.0, Some(borda()))).into() };
    let todas = app.etiquetas;
    let mostradas = if todas.len() > 4 { 3 } else { todas.len() };
    let mut chips: Vec<_> = todas[..mostradas].iter().map(|e| chip(e.to_string())).collect();
    if todas.len() > mostradas {
        chips.push(chip(format!("+{}", todas.len() - mostradas)));
    }
    chips
}

fn megas(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn pena_recolorida_nao_sobra_verde() {
        let svg = pena_na_cor("#7048b8");
        assert!(svg.contains("#7048b8") && !svg.contains("#287846") && !svg.contains("#72aa86"));
    }

    #[test]
    fn texto_sobre_contrasta() {
        assert_eq!(texto_sobre(Color::WHITE), Color::BLACK);
        assert_eq!(texto_sobre(Color::BLACK), Color::WHITE);
        assert_eq!(texto_sobre(hex(COR_LAUNCHER)), Color::WHITE);
    }

    /// A tela inteira desenha sem GPU, nos dois temas e nos dois modos, com e sem o menu aberto.
    #[test]
    fn tela_desenha() {
        for e in [true, false] {
            ESCURO.store(e, Ordering::Relaxed);
            let (mut l, _) = Launcher::novo(Tema::Escuro, false, true);
            for grade in [false, true] {
                l.grade = grade;
                l.menu = Some((0, Point::new(1200.0, 300.0)));
                let png = capture(&l.view(), TAMANHO_INICIAL, 1.0, &l.theme(), &mut Renderer::new(), None);
                assert!(png.len() > 10_000);
            }
            l.configuracoes = true;
            assert!(capture(&l.view(), TAMANHO_MINIMO, 1.0, &l.theme(), &mut Renderer::new(), None).len() > 10_000);
        }
    }
}
