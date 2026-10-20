//! `featherlauncher --bandeja`: a pena na bandeja do sistema, como a do OneDrive. Fica
//! aberta desde o login, confere as versões na entrada e a cada 6 horas, e avisa por
//! notificação; o clique (ou "Abrir o launcher" no menu) abre a janela.

/// shortcut: no Windows ainda não há ícone na bandeja (pede um laço de mensagens Win32);
/// lá o login só confere uma vez, avisa e sai. Fazer com a crate `tray-icon` quando pesar.
#[cfg(not(target_os = "linux"))]
pub fn rodar() {
    crate::aviso::verificar();
}

#[cfg(target_os = "linux")]
pub use linux::rodar;

#[cfg(target_os = "linux")]
mod linux {
    use crate::aviso;
    use crate::tarefas::no_tokio;
    use ksni::TrayMethods;
    use pelicano::futures::channel::mpsc::{unbounded, UnboundedSender};
    use pelicano::futures::executor::block_on;
    use pelicano::futures::future::select;
    use pelicano::futures::StreamExt;
    use std::time::Duration;

    const INTERVALO: Duration = Duration::from_secs(6 * 60 * 60);

    struct Bandeja {
        /// "FeatherSheet 0.1.4"…; vazio = tudo em dia.
        pendentes: Vec<String>,
        verificando: bool,
        /// Pede uma conferência fora de hora ("Verificar atualizações" no menu).
        pedir: UnboundedSender<()>,
    }

    impl ksni::Tray for Bandeja {
        fn id(&self) -> String {
            "featherlauncher".into()
        }

        fn title(&self) -> String {
            "FeatherWorks Launcher".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let Ok(img) = ::image::load_from_memory(include_bytes!("../pacote/icones/featherlauncher-64.png")) else { return Vec::new() };
            let img = img.to_rgba8();
            let (width, height) = img.dimensions();
            let mut data = img.into_raw();
            // RGBA → ARGB, a ordem que o protocolo pede.
            data.chunks_exact_mut(4).for_each(|p| p.rotate_right(1));
            vec![ksni::Icon { width: width as i32, height: height as i32, data }]
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "FeatherWorks Launcher".into(), description: self.situacao(), ..Default::default() }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            aviso::abrir_launcher();
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem { label: self.situacao(), enabled: false, ..Default::default() }.into(),
                ksni::MenuItem::Separator,
                StandardItem { label: "Abrir o launcher".into(), activate: Box::new(|_: &mut Self| aviso::abrir_launcher()), ..Default::default() }.into(),
                StandardItem {
                    label: "Verificar atualizações".into(),
                    enabled: !self.verificando,
                    activate: Box::new(|b: &mut Self| {
                        let _ = b.pedir.unbounded_send(());
                    }),
                    ..Default::default()
                }
                .into(),
                ksni::MenuItem::Separator,
                StandardItem { label: "Sair".into(), activate: Box::new(|_: &mut Self| std::process::exit(0)), ..Default::default() }.into(),
            ]
        }
    }

    impl Bandeja {
        fn situacao(&self) -> String {
            match self.pendentes.len() {
                _ if self.verificando => "Verificando atualizações…".into(),
                0 => "Tudo em dia".into(),
                1 => format!("Atualização: {}", self.pendentes[0]),
                n => format!("{n} atualizações disponíveis"),
            }
        }
    }

    pub fn rodar() {
        block_on(no_tokio(async {
            let (pedir, mut pedidos) = unbounded();
            let bandeja = match (Bandeja { pendentes: Vec::new(), verificando: true, pedir }).spawn().await {
                Ok(b) => b,
                // Sem bandeja (GNOME sem a extensão AppIndicator): ao menos o aviso.
                Err(_) => return aviso::avisar(&aviso::pendentes().await),
            };
            // Só avisa de novo quando a lista muda: a mesma atualização não aparece a cada 6 horas.
            let mut avisado: Vec<String> = Vec::new();
            loop {
                bandeja.update(|b| b.verificando = true).await;
                let pendentes = aviso::pendentes().await;
                bandeja.update(|b| (b.verificando, b.pendentes) = (false, pendentes.clone())).await;
                if pendentes != avisado {
                    avisado = pendentes.clone();
                    // O notify-send fica esperando o clique: fora do laço, para não segurar a próxima conferência.
                    tokio::task::spawn_blocking(move || aviso::avisar(&pendentes));
                }
                let _ = select(Box::pin(tokio::time::sleep(INTERVALO)), pedidos.next()).await;
            }
        }));
    }
}
