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

/// Toast do Windows pelo PowerShell, com o AppId do próprio PowerShell (o launcher
/// portátil não registra um). Os textos vão por variável de ambiente, fora do script.
#[cfg(windows)]
fn notificar(titulo: &str, corpo: &str) -> bool {
    const SCRIPT: &str = r#"
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null
$x = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
$t = $x.GetElementsByTagName('text')
$t.Item(0).AppendChild($x.CreateTextNode($env:FW_TITULO)) > $null
$t.Item(1).AppendChild($x.CreateTextNode($env:FW_CORPO)) > $null
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))
"#;
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]).env("FW_TITULO", titulo).env("FW_CORPO", corpo);
    let _ = instalar::sem_console(&mut cmd).status();
    false
}
