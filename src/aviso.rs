//! Conferir as versões sem janela e avisar por notificação do sistema: o que a bandeja
//! (`featherlauncher --bandeja`, aberta no login) faz.
use crate::instalar;
use crate::releases::{self, mais_nova, APPS};
use crate::tarefas::no_tokio;
use pelicano::futures::executor::block_on;
use pelicano::futures::future::join_all;
use std::process::Command;

/// Confere uma vez, avisa se houver o que atualizar e sai.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn verificar() {
    avisar(&block_on(no_tokio(pendentes())));
}

/// Notifica `pendentes` (nada se vazio); o clique em "Abrir o launcher" abre a janela.
pub fn avisar(pendentes: &[String]) {
    if pendentes.is_empty() {
        return;
    }
    let titulo = if pendentes.len() == 1 { "Atualização disponível" } else { "Atualizações disponíveis" };
    if notificar(titulo, &pendentes.join(", ")) {
        abrir_launcher();
    }
}

/// A janela do launcher, num processo à parte.
pub fn abrir_launcher() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).spawn();
    }
}

/// "FeatherSheet 0.1.4" de cada app instalado com versão mais nova publicada.
pub async fn pendentes() -> Vec<String> {
    let registro = instalar::instalados();
    let apps = APPS.iter().map(|app| {
        let registrada = registro.get(app.id).cloned();
        async move {
            // Instalado por fora (.deb, instalador do Windows): a versão vem de quem instalou.
            // Sem saber a versão, não avisa: "?" na tela vira "Atualizar", aqui seria aviso falso.
            let instalada = match registrada {
                Some(v) => v,
                None => tokio::task::spawn_blocking(move || instalar::executavel(app).and_then(|exe| instalar::versao_externa(app, &exe))).await.ok().flatten()?,
            };
            let rel = releases::ultima(app.id).await.ok()??;
            (rel.pacote.is_some() && mais_nova(&rel.versao, &instalada)).then(|| format!("{} {}", app.nome(), rel.versao))
        }
    });
    let launcher = async {
        let rel = releases::ultima("featherlauncher").await.ok()??;
        (rel.pacote.is_some() && mais_nova(&rel.versao, env!("CARGO_PKG_VERSION"))).then(|| format!("Launcher {}", rel.versao))
    };
    let (mut lista, launcher) = (join_all(apps).await, launcher.await);
    lista.push(launcher);
    lista.into_iter().flatten().collect()
}

/// Mostra a notificação; `true` se a pessoa clicou em "Abrir o launcher".
#[cfg(not(windows))]
fn notificar(titulo: &str, corpo: &str) -> bool {
    let base = |c: &mut Command| {
        c.args(["-a", "FeatherWorks Launcher", "-i", "featherlauncher"]);
    };
    let mut com_botao = Command::new("notify-send");
    base(&mut com_botao);
    // O `-A` espera a resposta e imprime o nome da ação escolhida.
    match com_botao.args(["-A", "abrir=Abrir o launcher", titulo, corpo]).output() {
        Ok(s) if s.status.success() => String::from_utf8_lossy(&s.stdout).trim() == "abrir",
        // Servidor sem botões: o notify-send já mostrou o aviso sem eles, mas sai com erro.
        Ok(s) if String::from_utf8_lossy(&s.stderr).contains("Displaying non-interactively") => false,
        // notify-send antigo, sem `-A`: só o aviso.
        _ => {
            let mut simples = Command::new("notify-send");
            base(&mut simples);
            let _ = simples.args([titulo, corpo]).status();
            false
        }
    }
}

/// Balão do ícone de notificação (Shell_NotifyIcon), que o Windows 10/11 mostra como
/// notificação comum. Nativo: sem PowerShell escondido, que o Defender acusava como trojan.
#[cfg(windows)]
fn notificar(titulo: &str, corpo: &str) -> bool {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_TIP, NIIF_LARGE_ICON, NIIF_USER, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, LoadIconW, HWND_MESSAGE};
    fn copiar(destino: &mut [u16], texto: &str) {
        // Corta no tamanho do campo, deixando o zero final.
        let cabe = destino.len() - 1;
        for (d, c) in destino.iter_mut().zip(texto.encode_utf16().take(cabe)) {
            *d = c;
        }
    }
    let classe: Vec<u16> = "STATIC\0".encode_utf16().collect();
    unsafe {
        // Uma janela só de mensagens: o ícone precisa de um dono.
        let janela = CreateWindowExW(0, classe.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
        // O ícone 1 do .exe (build.rs).
        let icone = LoadIconW(GetModuleHandleW(std::ptr::null()), 1 as _);
        let mut dados: NOTIFYICONDATAW = std::mem::zeroed();
        dados.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        dados.hWnd = janela;
        dados.uID = 1;
        dados.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
        dados.hIcon = icone;
        dados.hBalloonIcon = icone;
        dados.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        copiar(&mut dados.szTip, "FeatherWorks Launcher");
        copiar(&mut dados.szInfoTitle, titulo);
        copiar(&mut dados.szInfo, corpo);
        if Shell_NotifyIconW(NIM_ADD, &dados) != 0 {
            // O processo da bandeja sai logo depois; o ícone fica o tempo de a notificação aparecer.
            std::thread::sleep(std::time::Duration::from_secs(10));
            Shell_NotifyIconW(NIM_DELETE, &dados);
        }
        DestroyWindow(janela);
    }
    false
}
