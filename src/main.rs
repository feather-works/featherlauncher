#![cfg_attr(windows, windows_subsystem = "windows")]
//! FeatherWorks Launcher: instala, atualiza e abre os aplicativos da suíte.
mod instalar;
mod releases;

use feather_ui::cromo;
use feather_ui::dialogo::Dialogo;
use feather_ui::icones::icone;
use feather_ui::tema::{self, COR_BORDA, COR_BORDA_SUAVE, COR_ERRO, COR_PAINEL, COR_TEXTO, COR_TEXTO_MUTED};
use iced::widget::{button, image, column, container, horizontal_space, mouse_area, opaque, progress_bar, row, scrollable, stack, svg, text, text_input, Column, Space};
use iced::{event, font, keyboard, mouse, window, Event, Alignment, Border, Color, Element, Font, Length, Padding, Point, Size, Subscription, Task};
use instalar::Evento;
use releases::{mais_nova, App, Release, APPS};
use std::sync::{Mutex, PoisonError};

/// Última posição do mouse na janela: o menu "⋯" abre onde foi o clique.
static CURSOR: Mutex<Point> = Mutex::new(Point::ORIGIN);
/// Borda puxada no redimensionamento em curso (0 = nenhum; bits 1 = direita, 2 = baixo).
/// O Iced 0.13 não redimensiona janela sem moldura; a assinatura de eventos lê isto
/// para só mandar o movimento do mouse enquanto alguém puxa a borda.
static PUXANDO: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
const DIREITA: u8 = 1;
const BAIXO: u8 = 2;
const TAMANHO_MINIMO: Size = Size::new(960.0, 520.0);

const NEGRITO: Font = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };
const LARGURA_MENU: f32 = 236.0;
// Largura em que as etiquetas cabem ao lado da descrição, como no mockup.
const TAMANHO_INICIAL: Size = Size::new(1440.0, 860.0);
/// O canto da janela; a imagem de fundo é recortada nele (no Iced 0.13 a imagem não segue o raio do contêiner).
const RAIO_JANELA: f32 = 12.0;
/// Margem dos painéis até a borda da janela, e o tamanho deles: o desfoque do fundo é
/// desenhado nessas mesmas medidas, então o `view` usa estas constantes.
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

fn main() -> iced::Result {
    // Sobra da última atualização no Windows (o .exe em uso só pode ser renomeado).
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
    let (tema, grade) = ler_preferencias();
    aplicar_tema(tema);
    #[allow(unused_mut)]
    let mut janela = window::Settings {
        size: TAMANHO_INICIAL,
        // Os cantos arredondados precisam da janela transparente.
        transparent: true,
        min_size: Some(TAMANHO_MINIMO),
        // A barra de título é desenhada pelo app, como no FeatherSheet.
        decorations: false,
        // Alt+F4 passa pelo `Msg::Fechar`, que não fecha no meio de uma instalação.
        exit_on_close_request: false,
        ..Default::default()
    };
    #[cfg(target_os = "linux")]
    {
        janela.platform_specific.application_id = "featherlauncher".into();
    }
    iced::application("FeatherWorks Launcher", Launcher::update, Launcher::view)
        .theme(|_| tema::tema_iced())
        .window(janela)
        .subscription(Launcher::subscription)
        .run_with(move || Launcher::novo(tema, grade))
}

enum Consulta {
    Verificando,
    Falhou(String),
    /// `None` = sem release publicada ainda.
    Pronta(Option<Release>),
}

enum Download {
    Baixando { baixado: u64, total: u64, alca: iced::task::Handle },
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

/// Tema e modo de exibição (`true` = grade).
fn ler_preferencias() -> (Tema, bool) {
    let json: serde_json::Value = std::fs::read(arquivo_preferencias()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    (Tema::TODOS.iter().find(|t| json["tema"] == t.1).map_or(Tema::Automatico, |t| t.0), json["modo"] == "grade")
}

fn guardar_preferencias(t: Tema, grade: bool) {
    let caminho = arquivo_preferencias();
    let nome = Tema::TODOS.iter().find(|x| x.0 == t).map_or("automatico", |x| x.1);
    let _ = std::fs::create_dir_all(caminho.parent().unwrap());
    let _ = std::fs::write(caminho, serde_json::json!({ "tema": nome, "modo": if grade { "grade" } else { "lista" } }).to_string());
}

fn aplicar_tema(t: Tema) {
    let escuro = match t {
        Tema::Automatico => sistema_escuro().unwrap_or_else(|| tema::escuro_resolvido(true, false)),
        t => t == Tema::Escuro,
    };
    tema::definir_tema(escuro, 0);
}

/// O GNOME guarda o modo escuro em `color-scheme`, que a detecção do Iced não lê
/// (ela via claro com o sistema em `prefer-dark`). Fora do GNOME: `None`.
fn sistema_escuro() -> Option<bool> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let saida = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]).output().ok().filter(|s| s.status.success())?;
    let valor = String::from_utf8_lossy(&saida.stdout);
    // Só `prefer-dark` diz algo; `default` e `prefer-light` ficam com a detecção do Iced.
    valor.contains("prefer-dark").then_some(true)
}

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
    VersaoExterna(usize, Option<String>),
    Amostrar,
    LauncherRelease(Result<Option<Release>, String>),
    AtualizarLauncher,
    LauncherAtualizado(Result<std::path::PathBuf, String>),
    Janela(Option<window::Id>),
    Capturar,
    /// Começou a puxar a borda (`DIREITA`, `BAIXO` ou os dois).
    Puxar(u8),
    Puxando(Point),
    Soltou,
    Fotografada(window::Screenshot),
    Tamanho(Size),
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
    confirmar: Option<usize>,
    configuracoes: bool,
    tema: Tema,
    grade: bool,
    tamanho: Size,
    /// Tentou fechar no meio de uma instalação: o rodapé avisa.
    fechar_bloqueado: bool,
    launcher: Autoatualizacao,
    /// Memória do próprio launcher, para o rodapé.
    consumo: feather_ui::monitor::Monitor,
    /// A imagem de fundo decodificada, e a versão recortada para o tamanho atual da janela
    /// (um `Handle` novo por quadro subiria a imagem de novo; só muda ao redimensionar).
    fundo_original: ::image::RgbaImage,
    /// A mesma imagem desfocada, para o vidro fosco atrás do cabeçalho e do rodapé.
    fundo_desfocado: ::image::RgbaImage,
    fundo: image::Handle,
    /// CPU da última amostra e os tempos dela (processo, sistema).
    cpu: Option<f32>,
    cpu_antes: Option<(u64, u64)>,
    janela: Option<window::Id>,
    ultimo_clique_barra: Option<std::time::Instant>,
}

impl Launcher {
    fn novo(tema: Tema, grade: bool) -> (Self, Task<Msg>) {
        let mut l = Launcher {
            linhas: Vec::new(),
            filtro: Filtro::Todos,
            busca: String::new(),
            menu: None,
            confirmar: None,
            configuracoes: false,
            tema,
            grade,
            tamanho: TAMANHO_INICIAL,
            fechar_bloqueado: false,
            launcher: Autoatualizacao::Nada,
            consumo: feather_ui::monitor::Monitor::novo("FeatherWorks Launcher"),
            fundo_original: ::image::RgbaImage::default(),
            fundo_desfocado: ::image::RgbaImage::default(),
            fundo: image::Handle::from_rgba(1, 1, vec![0; 4]),
            cpu: None,
            cpu_antes: None,
            janela: None,
            ultimo_clique_barra: None,
        };
        let t = l.verificar();
        let mut l = l;
        l.fundo_original = ::image::load_from_memory(include_bytes!("../assets/fundo.jpg")).map(|i| i.to_rgba8()).unwrap_or_default();
        l.fundo_desfocado = desfocar(&l.fundo_original);
        l.fundo = fundo_arredondado(&l.fundo_original, &l.fundo_desfocado, l.tamanho);
        (l, Task::batch([t, window::get_latest().map(Msg::Janela), Task::perform(releases::ultima("featherlauncher"), Msg::LauncherRelease)]))
    }

    fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            window::resize_events().map(|(_, s)| Msg::Tamanho(s)),
            window::close_requests().map(|_| Msg::Fechar),
            iced::time::every(std::time::Duration::from_secs(1)).map(|_| Msg::Amostrar),
            // Guarda o cursor sem gerar mensagem: um `on_move` refazia a tela a cada pixel.
            event::listen_with(|e, _, _| {
                let puxando = PUXANDO.load(std::sync::atomic::Ordering::Relaxed) != 0;
                match e {
                    Event::Mouse(mouse::Event::CursorMoved { position }) => {
                        *CURSOR.lock().unwrap_or_else(PoisonError::into_inner) = position;
                        puxando.then_some(Msg::Puxando(position))
                    }
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if puxando => {
                        PUXANDO.store(0, std::sync::atomic::Ordering::Relaxed);
                        Some(Msg::Soltou)
                    }
                    _ => None,
                }
            }),
            keyboard::on_key_press(|k, _| matches!(k, keyboard::Key::Named(keyboard::key::Named::Escape)).then_some(Msg::Esc)),
        ])
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
            let versao = async move { tokio::task::spawn_blocking(move || instalar::executavel(app).and_then(|exe| instalar::versao_externa(app, &exe))).await.ok().flatten() };
            Task::perform(versao, move |v| Msg::VersaoExterna(i, v))
        });
        Task::batch(APPS.iter().enumerate().map(|(i, app)| Task::perform(releases::ultima(app.id), move |r| Msg::Release(i, r))).chain(externas))
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

    fn com_janela(&self, f: impl FnOnce(window::Id) -> Task<Msg>) -> Task<Msg> {
        self.janela.map_or_else(Task::none, f)
    }

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
            Msg::Menu(i) => self.menu = if self.menu.is_some_and(|(m, _)| m == i) { None } else { Some((i, *CURSOR.lock().unwrap_or_else(PoisonError::into_inner))) },
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
                let feito = async move { tokio::task::spawn_blocking(move || instalar::desinstalar(&APPS[i])).await.map_err(|e| e.to_string())? };
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
                self.consumo.amostrar();
                let agora = tempos_de_cpu();
                if let (Some((p, t)), Some((p0, t0))) = (agora, self.cpu_antes) {
                    self.cpu = Some((p.saturating_sub(p0) as f32 / t.saturating_sub(t0).max(1) as f32 * 100.0).clamp(0.0, 100.0));
                }
                self.cpu_antes = agora;
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
                return Task::perform(instalar::atualizar_launcher(rel), Msg::LauncherAtualizado);
            }
            Msg::LauncherAtualizado(r) => match r.and_then(|exe| std::process::Command::new(exe).spawn().map_err(|e| e.to_string())) {
                Ok(_) => std::process::exit(0),
                Err(e) => self.launcher = Autoatualizacao::Falhou(e),
            },
            Msg::Janela(id) => {
                self.janela = id;
                // FEATHERLAUNCHER_CAPTURAR=tela.png fotografa a janela depois que as releases chegam e fecha.
                if std::env::var_os("FEATHERLAUNCHER_CAPTURAR").is_some() {
                    return Task::perform(tokio::time::sleep(std::time::Duration::from_secs(4)), |_| Msg::Capturar);
                }
            }
            Msg::Puxar(borda) => PUXANDO.store(borda, std::sync::atomic::Ordering::Relaxed),
            Msg::Soltou => self.fundo = fundo_arredondado(&self.fundo_original, &self.fundo_desfocado, self.tamanho),
            Msg::Puxando(p) => {
                // A janela fica presa no canto de cima à esquerda: o tamanho é a posição do mouse.
                let borda = PUXANDO.load(std::sync::atomic::Ordering::Relaxed);
                let largura = if borda & DIREITA != 0 { p.x.max(TAMANHO_MINIMO.width) } else { self.tamanho.width };
                let altura = if borda & BAIXO != 0 { p.y.max(TAMANHO_MINIMO.height) } else { self.tamanho.height };
                return self.com_janela(|id| window::resize(id, Size::new(largura, altura)));
            }
            Msg::Capturar => return self.com_janela(|id| window::screenshot(id).map(Msg::Fotografada)),
            Msg::Fotografada(foto) => {
                let destino = std::env::var_os("FEATHERLAUNCHER_CAPTURAR").unwrap_or_default();
                if let Err(e) = ::image::save_buffer(&destino, &foto.bytes, foto.size.width, foto.size.height, ::image::ColorType::Rgba8) {
                    eprintln!("[captura] {e}");
                }
                std::process::exit(0);
            }
            Msg::Tamanho(s) => {
                self.tamanho = s;
                // Refazer o fundo a cada passo do arrasto travava a borda; enquanto puxa, a imagem só estica.
                if PUXANDO.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    self.fundo = fundo_arredondado(&self.fundo_original, &self.fundo_desfocado, s);
                }
            }
            Msg::Arrastar => {
                let Some(id) = self.janela else { return Task::none() };
                return feather_ui::janelas::arrastar_ou_maximizar(id, &mut self.ultimo_clique_barra);
            }
            Msg::Minimizar => return self.com_janela(|id| window::minimize(id, true)),
            Msg::Maximizar => return self.com_janela(window::toggle_maximize),
            Msg::Fechar => {
                // Fechar no meio mata a cópia dos arquivos e deixa o app pela metade.
                if self.linhas.iter().any(|l| matches!(l.download, Some(Download::Instalando | Download::Desinstalando))) {
                    self.fechar_bloqueado = true;
                    return Task::none();
                }
                return self.com_janela(window::close);
            }
        }
        Task::none()
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

    fn view(&self) -> Element<'_, Msg> {
        let nome = container(
            row![
                svg(svg::Handle::from_memory(pena_na_cor(COR_LAUNCHER).into_bytes())).width(28).height(28),
                text("FeatherWorks").size(16).font(NEGRITO),
                text("Launcher").size(15).color(COR_TEXTO_MUTED()),
                horizontal_space(),
                self.botao_launcher(),
                cromo::acao("settings", "Configurações", Some(Msg::Configuracoes)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([0, 14])
        .width(Length::Fill)
        .height(cromo::BARRA_ALTURA + 8.0)
        .align_y(Alignment::Center);
        // Sem a linha do `barra_integrada`: título e filtros formam um cartão só.
        let barra = row![
            mouse_area(nome).on_press(Msg::Arrastar),
            controles_janela(),
        ]
        .align_y(Alignment::Center);

        // Cabeçalho e rodapé flutuam em vidro; a lista é um cartão escuro quase opaco.
        let cabecalho = container(column![barra, self.faixa()]).height(ALTURA_CABECALHO).style(vidro(0.6));
        let corpo = column![cabecalho, self.conteudo(), self.rodape()].spacing(12).padding(MARGEM);
        // O fundo já vem na proporção da janela e com os cantos transparentes.
        let fundo = image(self.fundo.clone()).content_fit(iced::ContentFit::Fill).width(Length::Fill).height(Length::Fill);
        let base: Element<'_, Msg> = container(stack![fundo, corpo])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style { border: Border { color: COR_BORDA(), width: 1.0, radius: RAIO_JANELA.into() }, ..Default::default() })
            .into();
        let base: Element<'_, Msg> = stack![base, alcas_de_redimensionar()].into();

        if self.configuracoes {
            return self.janela_configuracoes(base);
        }
        match self.menu {
            Some((i, onde)) => stack![base, self.menu_mais(i, onde)].into(),
            None => base,
        }
    }

    /// Filtros em pílulas com a contagem, alternar lista/grade, busca e Verificar.
    fn faixa(&self) -> Element<'_, Msg> {
        let aba = |rotulo: &str, f: Filtro| {
            let n = self.contar(f);
            let destaque = f == Filtro::Atualizacoes && n > 0;
            let contagem = container(text(n.to_string()).size(11).font(NEGRITO).color(if destaque { Color::WHITE } else { COR_TEXTO() })).padding([1, 7]).style(move |_| container::Style {
                background: Some(if destaque { hex(LARANJA) } else { Color { a: 0.12, ..COR_TEXTO() } }.into()),
                border: Border { radius: 9.0.into(), ..Default::default() },
                ..Default::default()
            });
            let conteudo = row![text(rotulo.to_string()).size(13), contagem].spacing(8).align_y(Alignment::Center);
            button(conteudo).on_press(Msg::Filtrar(f)).padding([7, 14]).style(estilo_pilula(self.filtro == f))
        };
        let atualizaveis = self.contar(Filtro::Atualizacoes);
        let busca = container(
            row![traco(ICONE_BUSCA, 15.0, COR_TEXTO_MUTED()), text_input("Buscar aplicativo…", &self.busca).on_input(Msg::Buscar).size(13).padding(0).style(campo_sem_borda)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding([0, 12])
        .width(240)
        .height(34)
        .align_y(Alignment::Center)
        .style(|_| container::Style { background: Some(Color { a: 0.6, ..COR_PAINEL() }.into()), border: Border { color: COR_BORDA(), width: 1.0, radius: 8.0.into() }, ..Default::default() });

        let mut linha = row![
            aba("Todos", Filtro::Todos),
            aba("Instalados", Filtro::Instalados),
            aba("Atualizações", Filtro::Atualizacoes),
            aba("Não instalados", Filtro::NaoInstalados),
            horizontal_space(),
            self.alternar_modo(),
            busca,
            button(row![icone("refresh", 15.0), text("Verificar").size(13).font(NEGRITO)].spacing(6).align_y(Alignment::Center)).on_press(Msg::Verificar).style(estilo_pilula(false)).padding([8, 14]),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if atualizaveis > 0 {
            linha = linha.push(button(text(format!("Atualizar tudo ({atualizaveis})")).size(13).font(NEGRITO)).on_press(Msg::AtualizarTudo).style(estilo_botao_destaque).padding([8, 14]));
        }
        container(linha).padding(Padding { top: 4.0, right: 16.0, bottom: 12.0, left: 16.0 }).width(Length::Fill).into()
    }

    /// O que dizer da consulta no rodapé: verificando, ou a cota do GitHub estourada.
    /// "Atualizar launcher para X" na barra de título, só quando há versão nova.
    fn botao_launcher(&self) -> Element<'_, Msg> {
        let botao = |rotulo: String, m: Option<Msg>| button(text(rotulo).size(12).font(NEGRITO)).on_press_maybe(m).padding([4, 12]).style(estilo_botao_destaque);
        match &self.launcher {
            Autoatualizacao::Nada => Space::new(0, 0).into(),
            Autoatualizacao::Disponivel(rel) => botao(format!("Atualizar launcher para {}", rel.versao), Some(Msg::AtualizarLauncher)).into(),
            Autoatualizacao::Atualizando => botao("Atualizando o launcher…".into(), None).into(),
            Autoatualizacao::Falhou(e) => text(format!("Launcher não atualizou: {e}")).size(12).color(COR_ERRO()).into(),
        }
    }

    fn situacao(&self) -> Option<(String, Color)> {
        let limite = self.linhas.iter().find_map(|l| match &l.consulta {
            Consulta::Falhou(e) if e.starts_with(releases::LIMITE) => Some(e.clone()),
            _ => None,
        });
        let ocupado = self.linhas.iter().any(|l| matches!(l.download, Some(Download::Instalando | Download::Desinstalando)));
        if self.fechar_bloqueado && ocupado {
            return Some(("Espere terminar a instalação para fechar".into(), tom(LARANJA)));
        }
        match limite {
            Some(e) => Some((format!("Consulta pausada: {e}"), tom(LARANJA))),
            None if self.linhas.iter().any(|l| matches!(l.consulta, Consulta::Verificando)) => Some(("Verificando…".into(), COR_TEXTO_MUTED())),
            None => None,
        }
    }

    fn conteudo(&self) -> Element<'_, Msg> {
        let busca = self.busca.to_lowercase();
        let visiveis: Vec<usize> = (0..APPS.len())
            .filter(|&i| {
                let app = &APPS[i];
                self.passa(i, self.filtro)
                    && (app.nome().to_lowercase().contains(&busca) || app.descricao.to_lowercase().contains(&busca) || app.etiquetas.iter().any(|e| e.to_lowercase().contains(&busca)))
            })
            .collect();
        let vazio = || container(text("Nenhum aplicativo neste filtro.").size(13).color(COR_TEXTO_MUTED())).padding(22);
        if self.grade {
            let grade: Element<'_, Msg> = if visiveis.is_empty() {
                vazio().style(estilo_cartao).width(Length::Fill).into()
            } else {
                visiveis.iter().fold(row![].spacing(16), |r, &i| r.push(self.cartao(i))).wrap().into()
            };
            return self.rolagem(grade);
        }
        let cartao: Element<'_, Msg> = if visiveis.is_empty() {
            vazio().into()
        } else {
            let cab = |s: &'static str| text(s).size(11).font(NEGRITO).color(COR_TEXTO_MUTED());
            let cabecalho = row![
                cab("APLICATIVO").width(Length::Fill),
                cab("VERSÃO").width(COL_VERSAO),
                cab("ESTADO").width(COL_ESTADO),
                cab("ÚLTIMA ATUALIZAÇÃO").width(COL_DATA),
                container(cab("AÇÕES")).width(COL_ACOES).align_x(Alignment::End),
            ]
            .spacing(16)
            .padding([12, 20]);
            let mut lista = column![cabecalho];
            for &i in &visiveis {
                lista = lista.push(linha_fina(COR_BORDA_SUAVE())).push(self.linha(i));
            }
            lista.into()
        };
        self.rolagem(container(cartao).width(Length::Fill).style(estilo_cartao).into())
    }

    fn rolagem<'a>(&self, corpo: Element<'a, Msg>) -> Element<'a, Msg> {
        container(scrollable(container(corpo).padding([8, 12])))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style { text_color: Some(COR_TEXTO()), ..Default::default() })
            .into()
    }

    fn linha(&self, i: usize) -> Element<'_, Msg> {
        let (nome, col_versao) = self.partes(i);
        row![
            row![logo(&APPS[i], self.linhas[i].em_breve(), 48.0), nome].spacing(16).align_y(Alignment::Center).width(Length::Fill),
            col_versao.width(COL_VERSAO),
            container(self.estado(i)).width(COL_ESTADO),
            container(self.data(i)).width(COL_DATA),
            container(self.acoes(i)).width(COL_ACOES).align_x(Alignment::End),
        ]
        .spacing(16)
        .padding([14, 20])
        .align_y(Alignment::Center)
        .into()
    }

    /// O app no modo grade: o mesmo conteúdo da linha, empilhado num cartão.
    fn cartao(&self, i: usize) -> Element<'_, Msg> {
        let (nome, col_versao) = self.partes(i);
        container(
            column![
                row![logo(&APPS[i], self.linhas[i].em_breve(), 52.0), nome.width(Length::Fill)].spacing(14).align_y(Alignment::Center),
                row![col_versao, horizontal_space(), self.estado(i)].align_y(Alignment::Center),
                row![self.data(i), horizontal_space()].align_y(Alignment::Center),
                container(self.acoes(i)).width(Length::Fill).height(36).align_x(Alignment::End).align_y(Alignment::Center),
            ]
            .spacing(14),
        )
        .padding(20)
        .width(360)
        .style(estilo_cartao)
        .into()
    }

    /// Nome com descrição, etiquetas e avisos, e a coluna da versão: comuns à linha e ao cartão.
    fn partes(&self, i: usize) -> (Column<'_, Msg>, Column<'_, Msg>) {
        let app = &APPS[i];
        let l = &self.linhas[i];
        let rel = l.release();
        let apagado = l.em_breve();

        // Cada etiqueta entra solta na linha, para quebrar inteira (em grupo, o "+2" quebrava letra a letra).
        let descricao = etiquetas(app).into_iter().fold(row![text(app.descricao).size(13).color(COR_TEXTO_MUTED())], |r, e| r.push(e)).spacing(8).align_y(Alignment::Center).wrap();
        let mut nome = column![
            row![
                text("Feather").size(17).font(NEGRITO).color(if apagado { COR_TEXTO_MUTED() } else { COR_TEXTO() }),
                text(app.sufixo).size(17).font(NEGRITO).color(tom_app(app.cor)),
            ],
            descricao,
        ]
        .spacing(5);
        let falha = match &l.consulta {
            // O limite do GitHub sai uma vez só, no rodapé.
            Consulta::Falhou(e) if !e.starts_with(releases::LIMITE) => Some(e),
            _ => None,
        };
        if let Some(e) = l.erro.as_ref().or(falha) {
            nome = nome.push(text(e).size(12).color(COR_ERRO()));
        }
        if self.confirmar == Some(i) {
            let aviso = if instalar::pelo_sistema(app) { format!("Desinstalar {}? É pacote do sistema: ele vai pedir sua senha.", app.nome()) } else { format!("Desinstalar {}?", app.nome()) };
            nome = nome.push(text(aviso).size(12).color(COR_ERRO()));
        }

        let mono = |s: String| text(s).size(14);
        let versao: Element<'_, Msg> = match (l.atualizacao(), l.instalada.as_deref(), rel) {
            (Some(r), Some(inst), _) => row![
                mono(if inst == "?" { "?".into() } else { inst.to_string() }).color(COR_TEXTO_MUTED()),
                mono("→".into()).color(COR_TEXTO_MUTED()),
                mono(r.versao.clone()).color(tom(LARANJA)),
            ]
            .spacing(6)
            .into(),
            (None, Some(inst), _) if inst != "?" => mono(inst.to_string()).into(),
            (_, _, Some(r)) => mono(r.versao.clone()).into(),
            _ => mono("—".into()).color(COR_TEXTO_MUTED()).into(),
        };
        let mut col_versao = column![versao].spacing(4);
        col_versao = match rel {
            Some(_) => col_versao.push(
                button(text("Ver novidades ›").size(12).color(tom(AZUL)))
                    .on_press(Msg::Novidades(i))
                    .padding(0)
                    .style(|_, _| button::Style { background: None, ..Default::default() }),
            ),
            None if apagado => col_versao.push(text("Em desenvolvimento").size(12).color(COR_TEXTO_MUTED())),
            None => col_versao,
        };
        (nome, col_versao)
    }

    /// Data da última release, ou por que não há.
    fn data(&self, i: usize) -> Element<'_, Msg> {
        match self.linhas[i].release() {
            Some(r) => text(r.data.clone()).size(13).into(),
            None if self.linhas[i].em_breve() => text("Ainda sem versão publicada").size(12).color(COR_TEXTO_MUTED()).into(),
            None => text("—").size(13).color(COR_TEXTO_MUTED()).into(),
        }
    }

    fn alternar_modo(&self) -> Element<'_, Msg> {
        let botao = |nome: &str, dica: &str, grade: bool| {
            iced::widget::tooltip(
                button(icone(nome, 16.0)).on_press(Msg::Grade(grade)).padding([6, 8]).style(tema::estilo_aba_faixa(self.grade == grade)),
                container(text(dica.to_string()).size(12)).padding([4, 8]).style(tema::estilo_dica),
                iced::widget::tooltip::Position::Bottom,
            )
        };
        row![botao("list", "Lista", false), botao("layout-grid", "Grade", true)].spacing(2).padding(iced::Padding { right: 8.0, ..Default::default() }).into()
    }

    fn estado(&self, i: usize) -> Element<'_, Msg> {
        let l = &self.linhas[i];
        let (rotulo, cor) = match (&l.download, &l.consulta) {
            (Some(Download::Baixando { .. }), _) => ("Baixando…".to_string(), tom(AZUL)),
            (Some(Download::Pausado { .. }), _) => ("Pausado".into(), tom(LARANJA)),
            (Some(Download::Instalando), _) => ("Instalando…".into(), tom(AZUL)),
            (Some(Download::Desinstalando), _) => ("Desinstalando…".into(), tom(AZUL)),
            (None, Consulta::Verificando) if l.instalada.is_none() => ("Verificando…".into(), COR_TEXTO_MUTED()),
            (None, Consulta::Falhou(e)) if l.instalada.is_none() && e.starts_with(releases::LIMITE) => ("Aguardando o GitHub".into(), COR_TEXTO_MUTED()),
            (None, Consulta::Falhou(_)) if l.instalada.is_none() => ("Sem conexão".into(), COR_ERRO()),
            _ if l.atualizacao().is_some() => ("Atualização disponível".into(), tom(LARANJA)),
            _ if l.instalada.is_some() => ("Instalado".into(), tom(VERDE)),
            _ if l.em_breve() => return selo_em_breve(),
            _ if l.instalavel().is_some() => ("Não instalado".into(), COR_TEXTO_MUTED()),
            _ => (format!("Sem pacote para {}", releases::nome_do_sistema()), COR_TEXTO_MUTED()),
        };
        container(
            row![
                container(Space::new(8, 8)).style(move |_| container::Style { background: Some(cor.into()), border: Border { radius: 4.0.into(), ..Default::default() }, ..Default::default() }),
                text(rotulo).size(13).font(NEGRITO).color(cor),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([6, 14])
        .style(move |_| container::Style { background: Some(Color { a: 0.18, ..cor }.into()), border: Border { color: Color { a: 0.35, ..cor }, width: 1.0, radius: 16.0.into() }, ..Default::default() })
        .into()
    }

    fn acoes(&self, i: usize) -> Element<'_, Msg> {
        let l = &self.linhas[i];
        // Todo botão de ação tem o mesmo tamanho; o "⋯" e os de download são quadrados de 36.
        let primario = |conteudo: Element<'static, Msg>, m| button(container(conteudo).center(Length::Fill)).on_press(m).style(estilo_botao_destaque).width(LARGURA_BOTAO).height(36).padding(0);
        let quadrado = |desenho: Element<'static, Msg>, m| button(container(desenho).center(Length::Fill)).on_press(m).width(36).height(36).padding(0).style(estilo_botao_vidro);
        let barra = |baixado: u64, total: u64| {
            let pct = if total > 0 { baixado as f32 * 100.0 / total as f32 } else { 0.0 };
            column![
                text(format!("{pct:.0}% de {}", megas(total))).size(12).color(COR_TEXTO_MUTED()),
                progress_bar(0.0..=100.0, pct).height(6).style(|_| progress_bar::Style {
                    background: COR_BORDA_SUAVE().into(),
                    bar: hex(VERDE).into(),
                    border: Border { radius: 3.0.into(), ..Default::default() },
                }),
            ]
            .spacing(6)
            .width(Length::Fill)
        };
        let aviso = |s: &str| row![text(s.to_string()).size(12).color(COR_TEXTO_MUTED())];
        match &l.download {
            Some(Download::Baixando { baixado, total, .. }) => {
                row![barra(*baixado, *total), quadrado(icone("player-pause", 16.0).into(), Msg::Pausar(i)), quadrado(icone("x", 16.0).into(), Msg::Cancelar(i))]
            }
            Some(Download::Pausado { baixado, total }) => {
                row![barra(*baixado, *total), quadrado(icone("player-play", 16.0).into(), Msg::Instalar(i)), quadrado(icone("x", 16.0).into(), Msg::Cancelar(i))]
            }
            Some(Download::Instalando) => aviso("Conferindo o SHA256 e instalando…"),
            Some(Download::Desinstalando) => aviso("Removendo do computador…"),
            None if self.confirmar == Some(i) => {
                let perigo = button(text("Desinstalar").size(13).font(NEGRITO)).on_press(Msg::Desinstalar(i)).padding([9, 16]).style(|_, st| button::Style {
                    background: Some(if matches!(st, button::Status::Hovered | button::Status::Pressed) { hex("#a50e0e") } else { hex("#c5221f") }.into()),
                    text_color: Color::WHITE,
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                });
                let cancelar = button(text("Cancelar").size(13)).on_press(Msg::Esc).padding([9, 14]).style(tema::estilo_botao_secundario);
                row![cancelar, perigo]
            }
            None => {
                let mais = quadrado(traco(ICONE_MAIS, 16.0, COR_TEXTO()).into(), Msg::Menu(i));
                // Sem "⋯" (nada a fazer no menu), um vão do mesmo tamanho mantém as colunas.
                let vao = || Space::new(36, 36);
                let abrir = row![traco_cheio(ICONE_PLAY, 15.0, tinta_destaque()), text("Abrir").size(15).font(NEGRITO)].spacing(8).align_y(Alignment::Center);
                match (l.atualizacao(), l.instalada.is_some(), l.instalavel()) {
                    // Com atualização o "Abrir" encolhe para o ícone; a versão nova está na coluna Versão.
                    (Some(_), _, _) => row![
                        quadrado(traco_cheio(ICONE_PLAY, 14.0, COR_TEXTO()).into(), Msg::Abrir(i)),
                        button(container(text("Atualizar").size(13).font(NEGRITO)).center(Length::Fill)).on_press(Msg::Instalar(i)).style(estilo_botao_destaque).width(LARGURA_BOTAO - 44.0).height(36).padding(0),
                        mais
                    ],
                    (None, true, _) => row![primario(abrir.into(), Msg::Abrir(i)), mais],
                    (None, false, Some(r)) => {
                        let tamanho = r.pacote.as_ref().filter(|p| p.tamanho > 0).map_or(String::new(), |p| format!(" · {}", megas(p.tamanho)));
                        let rotulo = row![traco(ICONE_BAIXAR, 16.0, tinta_destaque()), text(format!("Instalar{tamanho}")).size(14).font(NEGRITO)].spacing(8).align_y(Alignment::Center);
                        row![primario(rotulo.into(), Msg::Instalar(i)), vao()]
                    }
                    _ if l.em_breve() => {
                        let rotulo = row![traco(ICONE_CADEADO, 15.0, COR_TEXTO_MUTED()), text("Indisponível").size(13).color(COR_TEXTO_MUTED())].spacing(8).align_y(Alignment::Center);
                        row![button(container(rotulo).center(Length::Fill)).style(estilo_botao_vidro).width(LARGURA_BOTAO).height(36).padding(0), vao()]
                    }
                    _ => row![],
                }
            }
        }
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// O menu do "⋯", aberto onde foi o clique; clicar fora fecha.
    fn menu_mais(&self, i: usize, onde: Point) -> Element<'_, Msg> {
        let item = |desenho: Element<'static, Msg>, rotulo: String, m, perigo: bool| {
            let cor = if perigo { COR_ERRO() } else { COR_TEXTO() };
            button(row![desenho, text(rotulo).size(13).color(cor)].spacing(10).align_y(Alignment::Center))
                .on_press(m)
                .width(Length::Fill)
                .padding([8, 10])
                .style(tema::estilo_item_menu(false, true))
        };
        let painel = container(
            column![
                item(icone("folder-open", 16.0).into(), "Mostrar pasta de instalação".into(), Msg::MostrarPasta(i), false),
                container(linha_fina(COR_BORDA_SUAVE())).padding([4, 0]),
                item(traco(ICONE_LIXO, 16.0, COR_ERRO()).into(), format!("Desinstalar {}…", APPS[i].nome()), Msg::PedirDesinstalar(i), true),
            ]
            .spacing(2),
        )
        .padding(6)
        .width(LARGURA_MENU)
        .style(tema::estilo_popup_menu);
        // Abre para baixo e para a esquerda do clique; perto da borda de baixo, para cima.
        let x = (onde.x - LARGURA_MENU + 18.0).clamp(8.0, (self.tamanho.width - LARGURA_MENU - 8.0).max(8.0));
        let y = if onde.y + 110.0 > self.tamanho.height { onde.y - 110.0 } else { onde.y + 18.0 };
        let posto = container(opaque(painel)).padding(Padding { top: y.max(0.0), left: x, right: 0.0, bottom: 0.0 });
        mouse_area(container(posto).width(Length::Fill).height(Length::Fill)).on_press(Msg::FecharMenu).into()
    }

    fn janela_configuracoes<'a>(&'a self, base: Element<'a, Msg>) -> Element<'a, Msg> {
        let opcoes = Tema::TODOS.iter().fold(row![].spacing(4), |r, &(t, _, rotulo)| {
            r.push(button(text(rotulo).size(13)).on_press(Msg::Tema(t)).padding([6, 14]).style(tema::estilo_aba_faixa(self.tema == t)))
        });
        let corpo = column![
            feather_ui::dialogo::secao("Aparência"),
            row![text("Tema").size(13).width(120), opcoes].align_y(Alignment::Center),
            feather_ui::dialogo::secao("Instalação"),
            feather_ui::dialogo::linha_rotulo_valor("Pasta", instalar::destino()),
            feather_ui::dialogo::linha_rotulo_valor("Versões de", format!("github.com/{}", releases::ORG)),
            feather_ui::dialogo::linha_rotulo_valor("Launcher", env!("CARGO_PKG_VERSION")),
        ]
        .spacing(10);
        Dialogo::new("Configurações", corpo).icone("settings").ao_fechar(Msg::Esc).sobre(base)
    }

    /// "RAM 84 MB · CPU 0,3%"; a CPU é da máquina inteira, como no monitor do sistema.
    fn texto_consumo(&self) -> String {
        let ram = format!("RAM {} MB", self.consumo.rss_kb().div_ceil(1024));
        match self.cpu {
            Some(cpu) => format!("{ram} · CPU {}%", format!("{cpu:.1}").replace('.', ",")),
            None => ram,
        }
    }

    fn rodape(&self) -> Element<'_, Msg> {
        let ponto = container(Space::new(8, 8)).style(|_| container::Style { background: Some(hex(VERDE).into()), border: Border { radius: 4.0.into(), ..Default::default() }, ..Default::default() });
        let muted = |s: String| text(s).size(12).color(COR_TEXTO_MUTED());
        let separador = || container(Space::new(1, 14)).style(|_| container::Style { background: Some(COR_BORDA().into()), ..Default::default() });
        let com_icone = |desenho: svg::Svg<'static>, s: String| row![desenho, muted(s)].spacing(8).align_y(Alignment::Center);
        let situacao: Element<'_, Msg> = match self.situacao() {
            Some((s, cor)) => text(s).size(12).color(cor).into(),
            None => com_icone(traco(ICONE_PACOTE, 14.0, COR_TEXTO_MUTED()), "Cada pacote é conferido pelo SHA256 da release".into()).into(),
        };
        container(
            row![
                row![ponto, muted(format!("Instalando para {}", releases::nome_do_sistema())), muted(instalar::destino().into())].spacing(8).align_y(Alignment::Center),
                separador(),
                situacao,
                horizontal_space(),
                muted(self.texto_consumo()),
                separador(),
                com_icone(traco(ICONE_GITHUB, 14.0, COR_TEXTO_MUTED()), format!("github.com/{}", releases::ORG)),
                separador(),
                muted(format!("Launcher {}", env!("CARGO_PKG_VERSION"))),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .padding([0, 20])
        .height(ALTURA_RODAPE)
        .width(Length::Fill)
        .align_y(Alignment::Center)
        .style(vidro(0.6))
        .into()
    }
}

// Traços Tabler que o catálogo do feather-ui não tem.
const ICONE_BAIXAR: &str = r#"<path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2M7 11l5 5 5-5M12 4v12"/>"#;
const ICONE_MAIS: &str = r#"<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>"#;
const ICONE_BUSCA: &str = r#"<circle cx="10" cy="10" r="7"/><path d="M21 21l-6-6"/>"#;
const ICONE_CADEADO: &str = r#"<path d="M5 13a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2zM11 16a1 1 0 1 0 2 0 1 1 0 0 0-2 0M8 11V7a4 4 0 1 1 8 0v4"/>"#;
const ICONE_RELOGIO: &str = r#"<path d="M3 12a9 9 0 1 0 18 0 9 9 0 0 0-18 0M12 7v5l3 3"/>"#;
const ICONE_PACOTE: &str = r#"<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3M12 12l8-4.5M12 12v9M12 12L4 7.5"/>"#;
const ICONE_GITHUB: &str = r#"<path d="M9 19c-4.3 1.4-4.3-2.5-6-3m12 5v-3.5c0-1 .1-1.4-.5-2 2.8-.3 5.5-1.4 5.5-6a4.6 4.6 0 0 0-1.3-3.2 4.2 4.2 0 0 0-.1-3.2s-1.1-.3-3.5 1.3a12.3 12.3 0 0 0-6.2 0C6.5 2.8 5.4 3.1 5.4 3.1a4.2 4.2 0 0 0-.1 3.2A4.6 4.6 0 0 0 4 9.5c0 4.6 2.7 5.7 5.5 6-.6.6-.6 1.2-.5 2V21"/>"#;
/// Desenhado cheio (não traço), como o ▶ do botão Abrir.
const ICONE_PLAY: &str = r#"<path d="M7 4v16l13-8z"/>"#;
const ICONE_LIXO: &str = r#"<path d="M4 7h16M10 11v6M14 11v6M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2l1-12M9 7V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"/>"#;

fn traco(caminho: &str, tamanho: f32, cor: Color) -> svg::Svg<'static> {
    let desenho = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{caminho}</svg>"##);
    svg(svg::Handle::from_memory(desenho.into_bytes())).width(tamanho).height(tamanho).style(move |_, _| svg::Style { color: Some(cor) })
}

fn traco_cheio(caminho: &str, tamanho: f32, cor: Color) -> svg::Svg<'static> {
    let desenho = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000" stroke="#000" stroke-width="2" stroke-linejoin="round">{caminho}</svg>"##);
    svg(svg::Handle::from_memory(desenho.into_bytes())).width(tamanho).height(tamanho).style(move |_, _| svg::Style { color: Some(cor) })
}

/// Até quatro etiquetas; com mais, três e um "+N".
fn etiquetas<'a>(app: &App) -> Vec<Element<'a, Msg>> {
    let chip = |s: String| -> Element<'a, Msg> {
        container(text(s).size(11).color(COR_TEXTO())).padding([3, 9]).style(|_| container::Style {
            background: Some(Color { a: 0.06, ..COR_TEXTO() }.into()),
            border: Border { color: COR_BORDA(), width: 1.0, radius: 6.0.into() },
            ..Default::default()
        })
        .into()
    };
    let todas = app.etiquetas;
    let mostradas = if todas.len() > 4 { 3 } else { todas.len() };
    let mut chips: Vec<_> = todas[..mostradas].iter().map(|e| chip(e.to_string())).collect();
    if todas.len() > mostradas {
        chips.push(chip(format!("+{}", todas.len() - mostradas)));
    }
    chips
}

/// Faixas finas na borda direita, na de baixo e no canto: puxar redimensiona. Esquerda e
/// topo ficam de fora porque mudariam a posição da janela, que o Wayland não deixa o app mudar.
fn alcas_de_redimensionar<'a>() -> Element<'a, Msg> {
    let alca = |w: Length, h: Length, borda: u8, cursor: mouse::Interaction| mouse_area(Space::new(w, h)).on_press(Msg::Puxar(borda)).interaction(cursor);
    stack![
        container(alca(Length::Fixed(6.0), Length::Fill, DIREITA, mouse::Interaction::ResizingHorizontally)).align_right(Length::Fill).height(Length::Fill),
        container(alca(Length::Fill, Length::Fixed(6.0), BAIXO, mouse::Interaction::ResizingVertically)).align_bottom(Length::Fill).width(Length::Fill),
        container(alca(Length::Fixed(16.0), Length::Fixed(16.0), DIREITA | BAIXO, mouse::Interaction::ResizingDiagonallyDown)).align_right(Length::Fill).align_bottom(Length::Fill),
    ]
    .into()
}

/// Minimizar, maximizar e fechar sem o fundo de barra do `cromo::controles_janela`, que
/// fazia um bloco cinza sobre o vidro do cabeçalho.
fn controles_janela<'a>() -> Element<'a, Msg> {
    let botao = |caminho: &str, msg: Msg, fechar: bool| {
        button(container(traco(caminho, 15.0, COR_TEXTO_MUTED())).center(Length::Fill)).on_press(msg).width(40).height(32).padding(0).style(move |_, st| button::Style {
            background: matches!(st, button::Status::Hovered | button::Status::Pressed)
                .then(|| if fechar { Color::from_rgb8(0xe8, 0x11, 0x23) } else { Color { a: 0.1, ..COR_TEXTO() } }.into()),
            border: Border { radius: 8.0.into(), ..Default::default() },
            ..Default::default()
        })
    };
    row![
        botao(r#"<path d="M5 12h14"/>"#, Msg::Minimizar, false),
        botao(r#"<rect x="5" y="5" width="14" height="14" rx="1"/>"#, Msg::Maximizar, false),
        botao(r#"<path d="M18 6L6 18M6 6l12 12"/>"#, Msg::Fechar, true),
    ]
    .spacing(4)
    .padding(Padding { right: 8.0, ..Default::default() })
    .into()
}

/// Desfoque em escala reduzida: o resultado é o mesmo a olho e sai em milissegundos.
fn desfocar(original: &::image::RgbaImage) -> ::image::RgbaImage {
    use ::image::imageops::{blur, resize, FilterType};
    let (w, h) = original.dimensions();
    if w < 8 || h < 8 {
        return original.clone();
    }
    let pequena = resize(original, w / 6, h / 6, FilterType::Triangle);
    resize(&blur(&pequena, 3.0), w, h, FilterType::Triangle)
}

/// Quanto do ponto (px, py) cai dentro do retângulo arredondado, de 0 a 1, com um pixel de suavização.
fn cobertura(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let dx = px - px.clamp(x0 + r, x1 - r);
    let dy = py - py.clamp(y0 + r, y1 - r);
    if px < x0 || px > x1 || py < y0 || py > y1 {
        return 0.0;
    }
    (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
}

/// O fundo recortado como o `ContentFit::Cover` faria, desfocado atrás do cabeçalho e do
/// rodapé (o Iced 0.13 não desfoca o que está atrás de um painel) e com os cantos da janela transparentes.
fn fundo_arredondado(original: &::image::RgbaImage, desfocado: &::image::RgbaImage, janela: Size) -> image::Handle {
    let (w, h) = original.dimensions();
    if w == 0 || h == 0 || janela.width < 1.0 || janela.height < 1.0 {
        return image::Handle::from_rgba(1, 1, vec![0; 4]);
    }
    let alvo = janela.width / janela.height;
    let (cw, ch) = if w as f32 / h as f32 > alvo { (((h as f32 * alvo) as u32).max(1), h) } else { (w, ((w as f32 / alvo) as u32).max(1)) };
    let (ox, oy) = ((w - cw) / 2, (h - ch) / 2);
    let mut img = ::image::imageops::crop_imm(original, ox, oy, cw, ch).to_image();
    // Tudo em pixels da imagem: k converte da janela.
    let k = ch as f32 / janela.height;
    let (iw, ih) = (cw as f32, ch as f32);
    let (m, rv) = (MARGEM * k, RAIO_VIDRO * k);
    let r = RAIO_JANELA * k;
    for (x, y, p) in img.enumerate_pixels_mut() {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let vidro = cobertura(px, py, m, m, iw - m, m + ALTURA_CABECALHO * k, rv)
            .max(cobertura(px, py, m, ih - m - ALTURA_RODAPE * k, iw - m, ih - m, rv));
        if vidro > 0.0 {
            let d = desfocado.get_pixel(ox + x, oy + y).0;
            for c in 0..3 {
                p.0[c] = (p.0[c] as f32 * (1.0 - vidro) + d[c] as f32 * vidro) as u8;
            }
        }
        let janela = cobertura(px, py, 0.0, 0.0, iw, ih, r);
        if janela < 1.0 {
            p.0[3] = (p.0[3] as f32 * janela) as u8;
        }
    }
    image::Handle::from_rgba(cw, ch, img.into_raw())
}

/// O fundo dos painéis: no escuro, um azul-marinho que conversa com o roxo da imagem
/// (o cinza do `COR_PAINEL` ficava chapado por cima dela).
fn cor_painel() -> Color {
    if tema::escuro() { Color::from_rgb8(17, 21, 31) } else { COR_PAINEL() }
}

/// O painel de vidro sobre a imagem de fundo. Sem desfoque no Iced 0.13, o vidro vem do
/// gradiente (mais claro no alto), do fio de luz na borda e da sombra que o descola do fundo.
fn vidro(alfa: f32) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| estilo_vidro(alfa, RAIO_VIDRO)
}

fn estilo_vidro(alfa: f32, raio: f32) -> container::Style {
    let base = cor_painel();
    let claro = Color { r: base.r + 0.06, g: base.g + 0.06, b: base.b + 0.08, a: alfa };
    let gradiente = iced::gradient::Linear::new(std::f32::consts::PI).add_stop(0.0, claro).add_stop(1.0, Color { a: alfa, ..base });
    container::Style {
        background: Some(iced::Background::Gradient(gradiente.into())),
        text_color: Some(COR_TEXTO()),
        border: Border { color: Color { a: 0.12, ..Color::WHITE }, width: 1.0, radius: raio.into() },
        shadow: iced::Shadow { color: Color { a: 0.45, ..Color::BLACK }, offset: iced::Vector::new(0.0, 8.0), blur_radius: 28.0 },
    }
}

/// O texto e o ícone sobre o botão de destaque.
fn tinta_destaque() -> Color {
    feather_ui::acessibilidade::texto_sobre(hex(COR_LAUNCHER))
}

/// Botão secundário em vidro (Indisponível, "⋯"): o cinza chapado do tema destoava dos painéis.
fn estilo_botao_vidro(_: &iced::Theme, st: button::Status) -> button::Style {
    let fundo = match st {
        button::Status::Hovered => 0.12,
        button::Status::Pressed => 0.16,
        button::Status::Disabled => 0.02,
        button::Status::Active => 0.05,
    };
    button::Style {
        background: Some(Color { a: fundo, ..Color::WHITE }.into()),
        text_color: if st == button::Status::Disabled { COR_TEXTO_MUTED() } else { COR_TEXTO() },
        border: Border { color: Color { a: 0.14, ..Color::WHITE }, width: 1.0, radius: 8.0.into() },
        ..Default::default()
    }
}

/// O botão principal: o ciano do acento em gradiente, com brilho; o `estilo_botao_primario`
/// do tema ficava azul ao passar o mouse.
fn estilo_botao_destaque(_: &iced::Theme, st: button::Status) -> button::Style {
    // O magenta da pena, o mesmo das penas do fundo; o ciano do acento do tema destoava.
    let acento = hex(COR_LAUNCHER);
    let clarear = |c: Color, k: f32| Color { r: c.r + (1.0 - c.r) * k, g: c.g + (1.0 - c.g) * k, b: c.b + (1.0 - c.b) * k, a: 1.0 };
    let escurecer = |c: Color, k: f32| Color { r: c.r * (1.0 - k), g: c.g * (1.0 - k), b: c.b * (1.0 - k), a: 1.0 };
    // Cor cheia: clarear o topo deixava o botão desbotado.
    let (topo, baixo) = match st {
        button::Status::Hovered => (clarear(acento, 0.12), acento),
        button::Status::Pressed => (escurecer(acento, 0.15), escurecer(acento, 0.15)),
        button::Status::Disabled => (Color { a: 0.4, ..acento }, Color { a: 0.4, ..acento }),
        button::Status::Active => (acento, escurecer(acento, 0.14)),
    };
    let gradiente = iced::gradient::Linear::new(std::f32::consts::PI).add_stop(0.0, topo).add_stop(1.0, baixo);
    button::Style {
        background: Some(iced::Background::Gradient(gradiente.into())),
        text_color: tinta_destaque(),
        border: Border { color: Color { a: 0.25, ..Color::WHITE }, width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow { color: Color { a: if st == button::Status::Hovered { 0.55 } else { 0.35 }, ..acento }, offset: iced::Vector::new(0.0, 2.0), blur_radius: 14.0 },
    }
}

/// Filtro em pílula: o escolhido com a borda e um véu da cor de destaque.
fn estilo_pilula(ativo: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, st| {
        let destaque = tema::COR_AZUL_ACENTO();
        let sobre = matches!(st, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(if ativo { Color { a: 0.16, ..destaque } } else { Color { a: if sobre { 0.75 } else { 0.5 }, ..COR_PAINEL() } }.into()),
            text_color: COR_TEXTO(),
            border: Border { color: if ativo { destaque } else { COR_BORDA() }, width: if ativo { 1.5 } else { 1.0 }, radius: 10.0.into() },
            ..Default::default()
        }
    }
}

/// A cor do nome do app, mais clara no escuro para ler sobre o vidro.
fn tom_app(cor: &str) -> Color {
    let c = hex(cor);
    if !tema::escuro() {
        return c;
    }
    let f = |v: f32| v + (1.0 - v) * 0.25;
    Color::from_rgb(f(c.r), f(c.g), f(c.b))
}

/// A logo do app; quem não tem logo no `feather-ui` ganha a pena da suíte na cor dele.
fn logo<'a>(app: &App, apagado: bool, tamanho: f32) -> Element<'a, Msg> {
    svg(svg::Handle::from_memory(pena_na_cor(app.cor).into_bytes())).width(tamanho).height(tamanho).opacity(if apagado { 0.55_f32 } else { 1.0 }).into()
}

/// A pena nos mesmos tons que o `marca::svg_para` usa, sem o quadrado de fundo.
fn pena_na_cor(cor: &str) -> String {
    let base = hex(cor);
    let c = |v: f32| ((v + (1.0 - v) * 0.40) * 255.0).round() as u8;
    let clara = format!("#{:02x}{:02x}{:02x}", c(base.r), c(base.g), c(base.b));
    let mut pena = feather_ui::marca::PENA_ORIGINAL.replace("#287846", cor).replace("#72aa86", &clara);
    if let Some(i) = pena.find("<rect id=\"fundo-arredondado\"") {
        let fim = pena[i..].find("/>").map_or(i, |f| i + f + 2);
        pena.replace_range(i..fim, "");
    }
    pena
}

/// (CPU do launcher, CPU da máquina), em ticks do `/proc`.
// shortcut: só Linux; no Windows o rodapé mostra só a RAM (GetProcessTimes quando o feather-ui publicar `Monitor::cpu`).
fn tempos_de_cpu() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let campos: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let processo = campos.get(11)?.parse::<u64>().ok()? + campos.get(12)?.parse::<u64>().ok()?;
    let total = std::fs::read_to_string("/proc/stat").ok()?.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse::<u64>().ok()).sum();
    Some((processo, total))
}

fn estilo_cartao(_: &iced::Theme) -> container::Style {
    // Quase opaco: o fundo atravessando as linhas atrapalhava a leitura.
    estilo_vidro(0.94, 12.0)
}

fn selo_em_breve<'a>() -> Element<'a, Msg> {
    container(row![traco(ICONE_RELOGIO, 15.0, COR_TEXTO()), text("Em breve").size(13).font(NEGRITO)].spacing(8).align_y(Alignment::Center))
        .padding([6, 14])
        .style(|_| container::Style { background: Some(Color { a: 0.06, ..COR_TEXTO() }.into()), border: Border { color: COR_BORDA(), width: 1.0, radius: 16.0.into() }, ..Default::default() })
        .into()
}

fn campo_sem_borda(t: &iced::Theme, s: text_input::Status) -> text_input::Style {
    text_input::Style { background: Color::TRANSPARENT.into(), border: Border::default(), ..text_input::default(t, s) }
}

fn linha_fina<'a>(cor: Color) -> Element<'a, Msg> {
    container(Space::with_height(1)).width(Length::Fill).style(move |_| container::Style { background: Some(cor.into()), ..Default::default() }).into()
}

/// As cores de estado legíveis no tema atual: no escuro, os mesmos tons mais claros.
fn tom(cor: &str) -> Color {
    if !tema::escuro() {
        return hex(cor);
    }
    hex(match cor {
        VERDE => "#81c995",
        LARANJA => "#f6ae6b",
        AZUL => "#8ab4f8",
        outra => outra,
    })
}

fn hex(s: &str) -> Color {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn megas(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod testes {
    #[test]
    fn pena_recolorida_nao_sobra_verde() {
        let svg = super::pena_na_cor("#7048b8");
        assert!(svg.contains("#7048b8") && !svg.contains("#287846") && !svg.contains("#72aa86"));
    }
}
