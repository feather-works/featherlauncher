//! FeatherWorks Launcher: instala, atualiza e abre os aplicativos da suíte.
mod instalar;
mod releases;

use feather_ui::cromo;
use feather_ui::dialogo::Dialogo;
use feather_ui::icones::icone;
use feather_ui::tema::{self, COR_BORDA, COR_BORDA_SUAVE, COR_ERRO, COR_FUNDO_JANELA, COR_PAINEL, COR_TEXTO, COR_TEXTO_MUTED};
use iced::widget::{button, column, container, horizontal_space, mouse_area, opaque, progress_bar, row, scrollable, stack, svg, text, text_input, Column, Space};
use iced::{font, keyboard, window, Alignment, Border, Color, Element, Font, Length, Padding, Point, Size, Subscription, Task};
use instalar::Evento;
use releases::{mais_nova, App, Release, APPS};

const NEGRITO: Font = Font { weight: font::Weight::Semibold, ..Font::DEFAULT };
const MONO: Font = Font::MONOSPACE;
const LARGURA_MENU: f32 = 236.0;
const LARANJA: &str = "#b45309";
const VERDE: &str = "#287846";
const AZUL: &str = "#1a56b8";

fn main() -> iced::Result {
    tema::definir_tema(tema::escuro_resolvido(true, false), 0);
    #[allow(unused_mut)]
    let mut janela = window::Settings {
        size: Size::new(1180.0, 720.0),
        min_size: Some(Size::new(960.0, 520.0)),
        // A barra de título é desenhada pelo app, como no FeatherSheet.
        decorations: false,
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
        .run_with(Launcher::novo)
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
    PedirDesinstalar(usize),
    Desinstalar(usize),
    Desinstalado(usize, Result<(), String>),
    Esc,
    Cursor(Point),
    Janela(Option<window::Id>),
    Tamanho(Size),
    Arrastar,
    Minimizar,
    Maximizar,
    Fechar,
}

struct Launcher {
    linhas: Vec<Linha>,
    filtro: Filtro,
    busca: String,
    /// O menu "⋯" aberto e onde o clique aconteceu (o menu abre ali).
    menu: Option<(usize, Point)>,
    confirmar: Option<usize>,
    cursor: Point,
    tamanho: Size,
    janela: Option<window::Id>,
    ultimo_clique_barra: Option<std::time::Instant>,
}

impl Launcher {
    fn novo() -> (Self, Task<Msg>) {
        let mut l = Launcher {
            linhas: Vec::new(),
            filtro: Filtro::Todos,
            busca: String::new(),
            menu: None,
            confirmar: None,
            cursor: Point::ORIGIN,
            tamanho: Size::new(1180.0, 720.0),
            janela: None,
            ultimo_clique_barra: None,
        };
        let t = l.verificar();
        (l, Task::batch([t, window::get_latest().map(Msg::Janela)]))
    }

    fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            window::resize_events().map(|(_, s)| Msg::Tamanho(s)),
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
                instalada: registro.get(app.id).cloned().or_else(|| instalar::executavel(app).map(|exe| instalar::versao_externa(app, &exe).unwrap_or_else(|| "?".into()))),
                // Download em andamento sobrevive ao "Verificar".
                download: antigas.next().and_then(|a| a.download),
                erro: None,
            })
            .collect();
        Task::batch(APPS.iter().enumerate().map(|(i, app)| Task::perform(releases::ultima(app.id), move |r| Msg::Release(i, r))))
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
                if let Some(p) = linha.release().and_then(|r| r.pacote.as_ref()) {
                    let _ = std::fs::remove_file(instalar::parcial(p));
                }
            }
            Msg::Abrir(i) => self.linhas[i].erro = instalar::abrir(&APPS[i]).err(),
            Msg::Filtrar(f) => self.filtro = f,
            Msg::Buscar(s) => self.busca = s,
            Msg::Menu(i) => self.menu = if self.menu.is_some_and(|(m, _)| m == i) { None } else { Some((i, self.cursor)) },
            Msg::FecharMenu => self.menu = None,
            Msg::MostrarPasta(i) => {
                self.menu = None;
                self.linhas[i].erro = instalar::mostrar_pasta(&APPS[i]).err();
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
            }
            Msg::Cursor(p) => self.cursor = p,
            Msg::Janela(id) => self.janela = id,
            Msg::Tamanho(s) => self.tamanho = s,
            Msg::Arrastar => {
                let Some(id) = self.janela else { return Task::none() };
                return feather_ui::janelas::arrastar_ou_maximizar(id, &mut self.ultimo_clique_barra);
            }
            Msg::Minimizar => return self.com_janela(|id| window::minimize(id, true)),
            Msg::Maximizar => return self.com_janela(window::toggle_maximize),
            Msg::Fechar => return self.com_janela(window::close),
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
            row![cromo::emblema(), text("FeatherWorks").size(13).font(NEGRITO), text("Launcher").size(13).color(COR_TEXTO_MUTED())]
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .padding([0, 12])
        .width(Length::Fill)
        .height(cromo::BARRA_ALTURA)
        .align_y(Alignment::Center)
        .style(tema::estilo_container_topo);
        let barra = cromo::barra_integrada(nome.into(), cromo::AcoesJanela { arrastar: Msg::Arrastar, minimizar: Msg::Minimizar, maximizar: Msg::Maximizar, fechar: Msg::Fechar });

        let corpo = column![barra, self.faixa(), self.conteudo(), self.rodape()];
        let base = cromo::moldura_janela(mouse_area(corpo).on_move(Msg::Cursor).into());

        if let Some(i) = self.confirmar {
            let nome = APPS[i].nome();
            let texto = text(format!("O {nome} sai deste computador. Suas planilhas, documentos e configurações não são apagados.")).size(13);
            return Dialogo::new(format!("Desinstalar o {nome}?"), texto)
                .icone("trash")
                .ao_fechar(Msg::Esc)
                .secundario("Cancelar", Some(Msg::Esc))
                .primario("Desinstalar", Some(Msg::Desinstalar(i)))
                .sobre(base);
        }
        match self.menu {
            Some((i, onde)) => stack![base, self.menu_mais(i, onde)].into(),
            None => base,
        }
    }

    /// Filtros no estilo das abas da faixa do FeatherSheet, busca e ações gerais.
    fn faixa(&self) -> Element<'_, Msg> {
        let aba = |rotulo: &str, f: Filtro| {
            let n = self.contar(f);
            let conteudo = row![text(rotulo.to_string()).size(13)].spacing(6).align_y(Alignment::Center);
            let conteudo = if f == Filtro::Atualizacoes && n > 0 {
                conteudo.push(
                    container(text(n.to_string()).size(11).font(NEGRITO).color(Color::WHITE))
                        .padding([1, 6])
                        .style(|_| container::Style { background: Some(hex(LARANJA).into()), border: Border { radius: 9.0.into(), ..Default::default() }, ..Default::default() }),
                )
            } else {
                conteudo.push(text(n.to_string()).size(13).color(COR_TEXTO_MUTED()))
            };
            button(conteudo).on_press(Msg::Filtrar(f)).padding([6, 12]).style(tema::estilo_aba_faixa(self.filtro == f))
        };
        let atualizaveis = self.contar(Filtro::Atualizacoes);
        let busca = container(
            row![traco(ICONE_BUSCA, 15.0, COR_TEXTO_MUTED()), text_input("Buscar aplicativo…", &self.busca).on_input(Msg::Buscar).size(13).padding(0).style(campo_sem_borda)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding([0, 10])
        .width(220)
        .height(30)
        .align_y(Alignment::Center)
        .style(|_| container::Style { background: Some(COR_PAINEL().into()), border: Border { color: COR_BORDA(), width: 1.0, radius: 4.0.into() }, ..Default::default() });

        let mut linha = row![
            aba("Todos", Filtro::Todos),
            aba("Instalados", Filtro::Instalados),
            aba("Atualizações", Filtro::Atualizacoes),
            aba("Não instalados", Filtro::NaoInstalados),
            horizontal_space(),
            busca,
            button(row![icone("refresh", 15.0), text("Verificar").size(13)].spacing(6).align_y(Alignment::Center)).on_press(Msg::Verificar).style(tema::estilo_botao_secundario).padding([6, 12]),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        if atualizaveis > 0 {
            linha = linha.push(button(text(format!("Atualizar tudo ({atualizaveis})")).size(13).font(NEGRITO)).on_press(Msg::AtualizarTudo).style(tema::estilo_botao_primario).padding([7, 14]));
        }
        column![
            container(linha).padding([8, 12]).width(Length::Fill).style(|_| container::Style { background: Some(COR_PAINEL().into()), ..Default::default() }),
            linha_fina(COR_BORDA()),
        ]
        .into()
    }

    fn conteudo(&self) -> Element<'_, Msg> {
        let verificando = self.linhas.iter().any(|l| matches!(l.consulta, Consulta::Verificando));
        let origem = if verificando { "Verificando…".to_string() } else { format!("Releases do GitHub ({})", releases::ORG) };
        let titulo = row![text("Aplicativos da suíte").size(20).font(NEGRITO), text(origem).size(12).color(COR_TEXTO_MUTED())].spacing(12).align_y(Alignment::End);

        let busca = self.busca.to_lowercase();
        let visiveis: Vec<usize> = (0..APPS.len())
            .filter(|&i| self.passa(i, self.filtro) && (APPS[i].nome().to_lowercase().contains(&busca) || APPS[i].descricao.to_lowercase().contains(&busca)))
            .collect();
        let cartao: Element<'_, Msg> = if visiveis.is_empty() {
            container(text("Nenhum aplicativo neste filtro.").size(13).color(COR_TEXTO_MUTED())).padding(22).into()
        } else {
            let mut lista = Column::new();
            for (n, &i) in visiveis.iter().enumerate() {
                if n > 0 {
                    lista = lista.push(linha_fina(COR_BORDA_SUAVE()));
                }
                lista = lista.push(self.linha(i));
            }
            lista.into()
        };
        let cartao = container(cartao).width(Length::Fill).style(|_| container::Style {
            background: Some(COR_PAINEL().into()),
            border: Border { color: COR_BORDA(), width: 1.0, radius: 10.0.into() },
            ..Default::default()
        });
        container(scrollable(column![titulo, cartao].spacing(14).padding(Padding { top: 28.0, right: 40.0, bottom: 20.0, left: 40.0 })))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style { background: Some(COR_FUNDO_JANELA().into()), text_color: Some(COR_TEXTO()), ..Default::default() })
            .into()
    }

    fn linha(&self, i: usize) -> Element<'_, Msg> {
        let app = &APPS[i];
        let l = &self.linhas[i];
        let rel = l.release();
        let apagado = l.em_breve();

        let mut nome = column![
            row![
                text("Feather").size(17).font(NEGRITO).color(if apagado { COR_TEXTO_MUTED() } else { COR_TEXTO() }),
                text(app.sufixo).size(17).font(NEGRITO).color(hex(app.cor)),
            ],
            text(app.descricao).size(13).color(COR_TEXTO_MUTED()),
        ]
        .spacing(3);
        let falha = match &l.consulta {
            Consulta::Falhou(e) => Some(e),
            _ => None,
        };
        if let Some(e) = l.erro.as_ref().or(falha) {
            nome = nome.push(text(e).size(12).color(COR_ERRO()));
        }

        let versao: Element<'_, Msg> = match (l.atualizacao(), l.instalada.as_deref(), rel) {
            (Some(r), Some(inst), _) => row![
                text(if inst == "?" { "versão desconhecida".to_string() } else { inst.to_string() }).size(13).font(MONO).color(COR_TEXTO_MUTED()),
                text("→").size(13).font(MONO).color(COR_TEXTO_MUTED()),
                text(r.versao.clone()).size(13).font(MONO).color(hex(LARANJA)),
            ]
            .spacing(6)
            .into(),
            (None, Some(inst), _) if inst != "?" => text(inst.to_string()).size(13).font(MONO).into(),
            (_, _, Some(r)) => text(r.versao.clone()).size(13).font(MONO).into(),
            _ => text("—").size(13).font(MONO).color(COR_TEXTO_MUTED()).into(),
        };
        let mut col_versao = column![versao].spacing(4);
        if let Some(r) = rel {
            col_versao = col_versao.push(text(format!("Lançada em {}", r.data)).size(12).color(COR_TEXTO_MUTED()));
        }

        row![
            logo(app, apagado),
            nome.width(Length::Fill),
            col_versao.width(170),
            container(self.estado(i)).width(190),
            container(self.acoes(i)).width(300).align_x(Alignment::End),
        ]
        .spacing(20)
        .padding([18, 22])
        .align_y(Alignment::Center)
        .into()
    }

    fn estado(&self, i: usize) -> Element<'_, Msg> {
        let l = &self.linhas[i];
        let (rotulo, cor) = match (&l.download, &l.consulta) {
            (Some(Download::Baixando { .. }), _) => ("Baixando…".to_string(), hex(AZUL)),
            (Some(Download::Pausado { .. }), _) => ("Pausado".into(), hex(LARANJA)),
            (Some(Download::Instalando), _) => ("Instalando…".into(), hex(AZUL)),
            (Some(Download::Desinstalando), _) => ("Desinstalando…".into(), hex(AZUL)),
            (None, Consulta::Verificando) if l.instalada.is_none() => ("Verificando…".into(), COR_TEXTO_MUTED()),
            (None, Consulta::Falhou(_)) if l.instalada.is_none() => ("Sem conexão".into(), COR_ERRO()),
            _ if l.atualizacao().is_some() => ("Atualização disponível".into(), hex(LARANJA)),
            _ if l.instalada.is_some() => ("Instalado".into(), hex(VERDE)),
            _ if l.em_breve() => return selo_em_breve(),
            _ if l.instalavel().is_some() => ("Não instalado".into(), COR_TEXTO_MUTED()),
            _ => (format!("Sem pacote para {}", releases::nome_do_sistema()), COR_TEXTO_MUTED()),
        };
        container(
            row![
                container(Space::new(7, 7)).style(move |_| container::Style { background: Some(cor.into()), border: Border { radius: 4.0.into(), ..Default::default() }, ..Default::default() }),
                text(rotulo).size(12).font(NEGRITO).color(cor),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([4, 10])
        .style(move |_| container::Style { background: Some(Color { a: 0.12, ..cor }.into()), border: Border { radius: 12.0.into(), ..Default::default() }, ..Default::default() })
        .into()
    }

    fn acoes(&self, i: usize) -> Element<'_, Msg> {
        let l = &self.linhas[i];
        let primario = |conteudo: Element<'static, Msg>, m| button(conteudo).on_press(m).style(tema::estilo_botao_primario).padding([9, 16]);
        let secundario = |s: &str, m| button(text(s.to_string()).size(13)).on_press(m).style(tema::estilo_botao_secundario).padding([9, 16]);
        let quadrado = |desenho: Element<'static, Msg>, m| button(container(desenho).center(Length::Fill)).on_press(m).width(36).height(36).padding(0).style(tema::estilo_botao_secundario);
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
            None => {
                let mais = quadrado(traco(ICONE_MAIS, 16.0, COR_TEXTO()).into(), Msg::Menu(i));
                match (l.atualizacao(), l.instalada.is_some(), l.instalavel()) {
                    (Some(r), _, _) => row![secundario("Abrir", Msg::Abrir(i)), primario(text(format!("Atualizar para {}", r.versao)).size(13).font(NEGRITO).into(), Msg::Instalar(i)), mais],
                    (None, true, _) => row![secundario("Abrir", Msg::Abrir(i)), mais],
                    (None, false, Some(r)) => {
                        let tamanho = r.pacote.as_ref().map_or(String::new(), |p| format!(" · {}", megas(p.tamanho)));
                        let rotulo = row![traco(ICONE_BAIXAR, 16.0, Color::WHITE), text(format!("Instalar{tamanho}")).size(13).font(NEGRITO)].spacing(8).align_y(Alignment::Center);
                        row![primario(rotulo.into(), Msg::Instalar(i))]
                    }
                    _ if l.em_breve() => aviso("Ainda sem versão publicada"),
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

    fn rodape(&self) -> Element<'_, Msg> {
        let destino = if cfg!(windows) { "%LOCALAPPDATA%\\Programs" } else if cfg!(target_os = "macos") { "~/Applications" } else { "~/.local" };
        let ponto = container(Space::new(7, 7)).style(|_| container::Style { background: Some(hex(VERDE).into()), border: Border { radius: 4.0.into(), ..Default::default() }, ..Default::default() });
        let muted = |s: String| text(s).size(12).color(COR_TEXTO_MUTED());
        column![
            linha_fina(COR_BORDA()),
            container(
                row![
                    row![ponto, muted(format!("Instalando para {} · {destino}", releases::nome_do_sistema()))].spacing(6).align_y(Alignment::Center),
                    muted("Cada pacote é conferido pelo SHA256 da release".into()),
                    horizontal_space(),
                    muted(format!("github.com/{}", releases::ORG)),
                    muted(format!("Launcher {}", env!("CARGO_PKG_VERSION"))),
                ]
                .spacing(16)
                .align_y(Alignment::Center),
            )
            .padding([0, 16])
            .height(34)
            .align_y(Alignment::Center)
            .style(|_| container::Style { background: Some(COR_PAINEL().into()), ..Default::default() }),
        ]
        .into()
    }
}

// Traços Tabler que o catálogo do feather-ui não tem.
const ICONE_BAIXAR: &str = r#"<path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2M7 11l5 5 5-5M12 4v12"/>"#;
const ICONE_MAIS: &str = r#"<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>"#;
const ICONE_BUSCA: &str = r#"<circle cx="10" cy="10" r="7"/><path d="M21 21l-6-6"/>"#;
const ICONE_LIXO: &str = r#"<path d="M4 7h16M10 11v6M14 11v6M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2l1-12M9 7V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"/>"#;

fn traco(caminho: &str, tamanho: f32, cor: Color) -> svg::Svg<'static> {
    let desenho = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{caminho}</svg>"##);
    svg(svg::Handle::from_memory(desenho.into_bytes())).width(tamanho).height(tamanho).style(move |_, _| svg::Style { color: Some(cor) })
}

/// A logo do app; quem não tem logo no `feather-ui` ganha a pena da suíte na cor dele.
fn logo<'a>(app: &App, apagado: bool) -> Element<'a, Msg> {
    let desenho = match app.produto {
        Some(p) => feather_ui::marca::icone_para(p, false, 52.0),
        None => svg(svg::Handle::from_memory(pena_na_cor(app.cor).into_bytes())).width(52).height(52),
    };
    desenho.opacity(if apagado { 0.55_f32 } else { 1.0 }).into()
}

/// Os mesmos tons que o `marca::svg_para` usa ao recolorir a pena.
fn pena_na_cor(cor: &str) -> String {
    let base = hex(cor);
    let claro = |f: f32| {
        let c = |v: f32| ((v + (1.0 - v) * f) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", c(base.r), c(base.g), c(base.b))
    };
    feather_ui::marca::PENA_ORIGINAL
        .replace("#287846", cor)
        .replace("#72aa86", &claro(0.40))
        .replace("#f4faf5", &claro(0.95))
        .replace("#e8f4eb", &claro(0.88))
        .replace("#e4f0e7", &claro(0.85))
}

fn selo_em_breve<'a>() -> Element<'a, Msg> {
    container(text("Em breve").size(12).font(NEGRITO).color(COR_TEXTO_MUTED()))
        .padding([4, 10])
        .style(|_| container::Style { border: Border { color: COR_BORDA(), width: 1.0, radius: 12.0.into() }, ..Default::default() })
        .into()
}

fn campo_sem_borda(t: &iced::Theme, s: text_input::Status) -> text_input::Style {
    text_input::Style { background: Color::TRANSPARENT.into(), border: Border::default(), ..text_input::default(t, s) }
}

fn linha_fina<'a>(cor: Color) -> Element<'a, Msg> {
    container(Space::with_height(1)).width(Length::Fill).style(move |_| container::Style { background: Some(cor.into()), ..Default::default() }).into()
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
