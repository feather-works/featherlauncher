//! FeatherWorks Launcher: instala, atualiza e abre os aplicativos da suíte.
mod instalar;
mod releases;

use feather_ui::tema::{self, COR_BORDA, COR_ERRO, COR_FUNDO_JANELA, COR_PAINEL, COR_TEXTO, COR_TEXTO_MUTED};
use iced::widget::{button, column, container, horizontal_space, progress_bar, row, scrollable, svg, text, text_input, Column};
use iced::{font, Alignment, Border, Color, Element, Font, Length, Task};
use instalar::Evento;
use releases::{mais_nova, Release, APPS};

const NEGRITO: Font = Font { weight: font::Weight::Bold, ..Font::DEFAULT };

fn main() -> iced::Result {
    tema::definir_tema(tema::escuro_resolvido(true, false), 0);
    iced::application("FeatherWorks Launcher", Launcher::update, Launcher::view)
        .theme(|_| tema::tema_iced())
        .window_size((1120.0, 600.0))
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
}

struct Launcher {
    linhas: Vec<Linha>,
    filtro: Filtro,
    busca: String,
}

impl Launcher {
    fn novo() -> (Self, Task<Msg>) {
        let mut l = Launcher { linhas: Vec::new(), filtro: Filtro::Todos, busca: String::new() };
        let t = l.verificar();
        (l, t)
    }

    fn verificar(&mut self) -> Task<Msg> {
        let registro = instalar::instalados();
        let antigas = std::mem::take(&mut self.linhas);
        let mut antigas = antigas.into_iter();
        self.linhas = APPS
            .iter()
            .map(|app| Linha {
                consulta: Consulta::Verificando,
                instalada: registro.get(app.id).cloned().or_else(|| instalar::executavel(app).map(|_| "?".into())),
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
        if matches!(linha.download, Some(Download::Baixando { .. } | Download::Instalando)) {
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

    fn view(&self) -> Element<'_, Msg> {
        let topo = row![
            svg(svg::Handle::from_memory(feather_ui::marca::PENA_ORIGINAL.as_bytes())).width(28).height(28),
            text("FeatherWorks").size(20).font(NEGRITO),
            selo("LAUNCHER", COR_TEXTO(), true),
            text("Suíte de escritório").size(12).color(COR_TEXTO_MUTED()),
            horizontal_space(),
            button(text("Verificar de novo").size(13)).on_press(Msg::Verificar).style(tema::estilo_botao_secundario).padding([6, 14]),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let contar = |f| (0..self.linhas.len()).filter(|&i| self.passa(i, f)).count();
        let chip = |rotulo: &str, f: Filtro| {
            let ativo = self.filtro == f;
            button(text(format!("{rotulo} {}", contar(f))).size(12))
                .on_press(Msg::Filtrar(f))
                .padding([6, 12])
                .style(if ativo { tema::estilo_botao_primario } else { tema::estilo_botao_secundario })
        };
        let atualizaveis = contar(Filtro::Atualizacoes);
        let mut filtros = row![
            chip("Todos", Filtro::Todos),
            chip("Instalados", Filtro::Instalados),
            chip("Atualizações", Filtro::Atualizacoes),
            chip("Não instalados", Filtro::NaoInstalados),
            horizontal_space(),
            text_input("Buscar aplicativo", &self.busca).on_input(Msg::Buscar).width(240).padding(7).size(13),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if atualizaveis > 0 {
            filtros = filtros.push(button(text(format!("Atualizar tudo ({atualizaveis})")).size(13)).on_press(Msg::AtualizarTudo).style(tema::estilo_botao_primario).padding([7, 14]));
        }

        let cab = |s: &'static str| text(s).size(11).color(COR_TEXTO_MUTED());
        let cabecalho = colunas([cab("APLICATIVO").into(), cab("INSPIRADO EM").into(), cab("ESTADO").into(), cab("INSTALADA").into(), cab("MAIS NOVA").into(), cab("LANÇADA").into(), text("").into()]);

        let busca = self.busca.to_lowercase();
        let mut tabela = Column::new().push(cabecalho).push(separador());
        for (i, app) in APPS.iter().enumerate() {
            if !self.passa(i, self.filtro) || !(app.nome().to_lowercase().contains(&busca) || app.descricao.to_lowercase().contains(&busca)) {
                continue;
            }
            tabela = tabela.push(self.linha(i)).push(separador());
        }

        let rodape = row![
            text(format!("Fonte: releases do GitHub ({})", releases::ORG)).size(11).color(COR_TEXTO_MUTED()),
            horizontal_space(),
            text(format!("Instalando para {}", releases::nome_do_sistema())).size(11).color(COR_TEXTO_MUTED()),
        ];

        let painel = container(scrollable(tabela)).height(Length::Fill).style(|_| container::Style {
            background: Some(COR_PAINEL().into()),
            border: Border { color: COR_BORDA(), width: 1.0, radius: 8.0.into() },
            ..Default::default()
        });
        container(column![topo, filtros, painel, rodape].spacing(14))
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style { background: Some(COR_FUNDO_JANELA().into()), text_color: Some(COR_TEXTO()), ..Default::default() })
            .into()
    }

    fn linha(&self, i: usize) -> Element<'_, Msg> {
        let app = &APPS[i];
        let l = &self.linhas[i];
        let rel = l.release();

        let mut nome = column![
            row![text("Feather").size(15).font(NEGRITO), text(app.sufixo).size(15).font(NEGRITO).color(hex(app.cor))],
            text(app.descricao).size(12).color(COR_TEXTO_MUTED()),
        ];
        let falha = match &l.consulta {
            Consulta::Falhou(e) => Some(e),
            _ => None,
        };
        if let Some(e) = l.erro.as_ref().or(falha) {
            nome = nome.push(text(e).size(11).color(COR_ERRO()));
        }
        let app_cel = row![feather_ui::marca::icone_para(app.produto, false, 34.0), nome].spacing(10).align_y(Alignment::Center);

        let (estado, cor) = match (&l.download, &l.consulta) {
            (Some(Download::Baixando { .. }), _) => ("BAIXANDO…".to_string(), tema::COR_AZUL_ACENTO()),
            (Some(Download::Pausado { .. }), _) => ("PAUSADO".into(), hex("#e8710a")),
            (Some(Download::Instalando), _) => ("INSTALANDO…".into(), tema::COR_AZUL_ACENTO()),
            (None, Consulta::Verificando) => ("VERIFICANDO…".into(), COR_TEXTO_MUTED()),
            (None, Consulta::Falhou(_)) if l.instalada.is_none() => ("SEM CONEXÃO".into(), COR_ERRO()),
            _ if l.atualizacao().is_some() => ("ATUALIZAÇÃO DISPONÍVEL".into(), hex("#e8710a")),
            _ if l.instalada.is_some() => ("INSTALADO · EM DIA".into(), hex("#188038")),
            (None, Consulta::Pronta(None)) => ("EM BREVE".into(), COR_TEXTO_MUTED()),
            _ if l.instalavel().is_some() => ("DISPONÍVEL".into(), tema::COR_AZUL_ACENTO()),
            _ => (format!("SEM PACOTE PARA {}", releases::nome_do_sistema().to_uppercase()), COR_TEXTO_MUTED()),
        };

        let instalada = l.instalada.as_deref().map_or("—", |v| if v == "?" { "?" } else { v });
        let nova = rel.map_or("—".to_string(), |r| r.versao.clone());
        let cor_nova = if l.atualizacao().is_some() { hex("#e8710a") } else { COR_TEXTO() };
        let data = rel.map_or("—".to_string(), |r| r.data.clone());

        colunas([
            app_cel.into(),
            text(app.inspirado).size(13).color(COR_TEXTO_MUTED()).into(),
            selo(&estado, cor, false).into(),
            text(instalada).size(13).font(Font::MONOSPACE).into(),
            text(nova).size(13).font(Font::MONOSPACE).color(cor_nova).into(),
            text(data).size(13).color(COR_TEXTO_MUTED()).into(),
            self.acoes(i),
        ])
    }

    fn acoes(&self, i: usize) -> Element<'_, Msg> {
        let l = &self.linhas[i];
        let primario = |s: String, m| button(text(s).size(12).width(Length::Fill).center()).on_press(m).style(tema::estilo_botao_primario).padding([7, 10]).width(Length::Fill);
        let secundario = |s: &str, m| button(text(s.to_string()).size(12).center()).on_press(m).style(tema::estilo_botao_secundario).padding([7, 10]);
        let barra = |baixado: u64, total: u64| {
            let pct = if total > 0 { baixado as f32 * 100.0 / total as f32 } else { 0.0 };
            column![
                text(format!("{pct:.0}% de {}", megas(total))).size(11).color(COR_TEXTO_MUTED()),
                progress_bar(0.0..=100.0, pct).height(5),
            ]
            .spacing(4)
            .width(Length::Fill)
        };
        match &l.download {
            Some(Download::Baixando { baixado, total, .. }) => row![barra(*baixado, *total), secundario("Pausar", Msg::Pausar(i)), secundario("✕", Msg::Cancelar(i))],
            Some(Download::Pausado { baixado, total }) => row![barra(*baixado, *total), secundario("Continuar", Msg::Instalar(i)), secundario("✕", Msg::Cancelar(i))],
            Some(Download::Instalando) => row![text("Conferindo e instalando…").size(12).color(COR_TEXTO_MUTED())],
            None => match (l.atualizacao(), l.instalada.is_some(), l.instalavel()) {
                (Some(r), _, _) => row![primario(format!("Atualizar para {}", r.versao), Msg::Instalar(i)), secundario("Abrir", Msg::Abrir(i))],
                (None, true, _) => row![secundario("Abrir", Msg::Abrir(i)).width(Length::Fill)],
                (None, false, Some(_)) => row![primario("Instalar".into(), Msg::Instalar(i))],
                _ => row![],
            },
        }
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    }
}

/// As sete colunas da tabela, com as mesmas larguras no cabeçalho e nas linhas.
fn colunas<'a>(celulas: [Element<'a, Msg>; 7]) -> Element<'a, Msg> {
    let larguras = [Length::FillPortion(3), Length::Fixed(170.0), Length::Fixed(190.0), Length::Fixed(80.0), Length::Fixed(80.0), Length::Fixed(100.0), Length::Fixed(250.0)];
    let mut r = row![].spacing(12).padding([10, 16]).align_y(Alignment::Center);
    for (c, w) in celulas.into_iter().zip(larguras) {
        r = r.push(container(c).width(w));
    }
    r.into()
}

fn separador<'a>() -> Element<'a, Msg> {
    container(text("")).height(1).width(Length::Fill).style(|_| container::Style { background: Some(tema::COR_BORDA_SUAVE().into()), ..Default::default() }).into()
}

fn selo<'a>(rotulo: &str, cor: Color, cheio: bool) -> container::Container<'a, Msg> {
    let fundo = if cheio { cor } else { Color { a: 0.10, ..cor } };
    let letra = if cheio { COR_PAINEL() } else { cor };
    container(text(rotulo.to_string()).size(10).font(NEGRITO).color(letra)).padding([3, 7]).style(move |_| container::Style {
        background: Some(fundo.into()),
        border: Border { color: Color { a: 0.5, ..cor }, width: 1.0, radius: 3.0.into() },
        ..Default::default()
    })
}

fn hex(s: &str) -> Color {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn megas(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}
