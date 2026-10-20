//! Baixar, conferir o SHA256, instalar e abrir. Tudo na pasta do usuário,
//! sem pedir senha de administrador.
use crate::releases::{cliente, App, Arquivo, Release};
use pelicano::futures::channel::mpsc::Sender;
use pelicano::futures::{SinkExt, Stream};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, PoisonError};

#[derive(Debug, Clone)]
pub enum Evento {
    Progresso { baixado: u64, total: u64 },
    Instalando,
    /// A versão instalada, ou o motivo da falha.
    Fim(Result<String, String>),
}

fn pasta_dados() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("featherworks")
}

fn pasta_downloads() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("featherworks")
}

/// Onde os apps vão parar, como a tela mostra.
pub fn destino() -> &'static str {
    if cfg!(windows) { "%LOCALAPPDATA%\\Programs" } else if cfg!(target_os = "macos") { "~/Applications" } else { "~/.local" }
}

/// O arquivo baixado pela metade fica aqui; é dele que a pausa continua. A versão
/// vai no nome porque o pacote não a tem: o resto da 0.1.0 não emenda na 0.1.1.
pub fn parcial(pacote: &Arquivo, versao: &str) -> PathBuf {
    pasta_downloads().join(format!("{versao}-{}.parcial", pacote.nome))
}

/// Versões instaladas pelo launcher: `{"feathersheet": "0.1.3"}`.
pub fn instalados() -> HashMap<String, String> {
    ler_instalados(&pasta_dados())
}

fn ler_instalados(pasta: &Path) -> HashMap<String, String> {
    fs::read(pasta.join("instalados.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// "Atualizar tudo" instala em paralelo: sem a trava, duas instalações que acabam
/// juntas leem o mesmo registro e a segunda gravação apaga a linha da primeira.
static REGISTRO: Mutex<()> = Mutex::new(());

fn alterar_instalados(pasta: &Path, mudar: impl FnOnce(&mut HashMap<String, String>)) -> Result<(), String> {
    let _trava = REGISTRO.lock().unwrap_or_else(PoisonError::into_inner);
    let mut mapa = ler_instalados(pasta);
    mudar(&mut mapa);
    fs::create_dir_all(pasta).map_err(|e| e.to_string())?;
    // Grava ao lado e renomeia: uma queda no meio não deixa o registro pela metade.
    let tmp = pasta.join("instalados.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&mapa).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, pasta.join("instalados.json")).map_err(|e| e.to_string())
}

/// Onde o app instalado está (pelo launcher, pelo `instalar.sh`, pelo `.deb` ou pelo instalador do Windows).
pub fn executavel(app: &App) -> Option<PathBuf> {
    let casa = dirs::home_dir().unwrap_or_default();
    let candidatos: Vec<PathBuf> = if cfg!(windows) {
        let local = dirs::data_local_dir().unwrap_or_default();
        vec![local.join("Programs").join(app.nome()).join(format!("{}.exe", app.id))]
    } else if cfg!(target_os = "macos") {
        vec![casa.join("Applications").join(format!("{}.app", app.nome())), PathBuf::from(format!("/Applications/{}.app", app.nome()))]
    } else {
        vec![casa.join(".local/bin").join(app.id), Path::new("/usr/bin").join(app.id)]
    };
    candidatos.into_iter().find(|c| c.exists())
}

/// A versão de um app que o launcher não instalou, perguntada a quem instalou
/// (.deb/.rpm, instalador do Windows, Info.plist do .app). Rodar o executável
/// com `--version` não serve: ele abriria a janela.
pub fn versao_externa(app: &App, exe: &Path) -> Option<String> {
    let saida = |cmd: &mut Command| sem_console(cmd).output().ok().filter(|s| s.status.success()).map(|s| String::from_utf8_lossy(&s.stdout).trim().to_string()).filter(|s| !s.is_empty());
    let v = if cfg!(windows) {
        let chave = format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", app.nome());
        saida(Command::new("reg").args(["query", &chave, "/v", "DisplayVersion"]))?.split_whitespace().last()?.to_string()
    } else if cfg!(target_os = "macos") {
        saida(Command::new("defaults").arg("read").arg(exe.join("Contents/Info")).arg("CFBundleShortVersionString"))?
    } else {
        saida(Command::new("dpkg-query").args(["-W", "-f=${Version}", app.id])).or_else(|| saida(Command::new("rpm").args(["-q", "--qf", "%{VERSION}", app.id])))?
    };
    limpar_versao(&v)
}

/// "0.1.3-1" (revisão do pacote) e "1:0.1.3" (época) → "0.1.3".
fn limpar_versao(v: &str) -> Option<String> {
    let v = v.rsplit(':').next()?.split(['-', '~', '+']).next()?;
    v.starts_with(|c: char| c.is_ascii_digit()).then(|| v.to_string())
}

pub fn abrir(app: &App) -> Result<(), String> {
    let exe = executavel(app).ok_or("não encontrei o aplicativo instalado")?;
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(&exe);
        c
    } else {
        Command::new(&exe)
    };
    // No Linux, num escopo próprio como o menu do GNOME faz (`app-gnome-<id>-…`): sem
    // isso o app fica dentro do launcher e o Recursos não o mostra com logo e nome.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let unidade = format!("--unit=app-gnome-{}-{}", app.id, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        if Command::new("systemd-run").args(["--user", "--scope", "--quiet", "--collect", &unidade]).arg(&exe).spawn().is_ok() {
            return Ok(());
        }
    }
    cmd.spawn().map(|_| ()).map_err(|e| format!("não abriu: {e}"))
}

/// Abre a pasta onde o app está no gerenciador de arquivos do sistema.
pub fn mostrar_pasta(app: &App) -> Result<(), String> {
    let exe = executavel(app).ok_or("não encontrei o aplicativo instalado")?;
    abrir_externo(exe.parent().unwrap_or(&exe).as_os_str())
}

/// Página ou pasta no programa padrão do sistema (navegador, gerenciador de arquivos).
pub fn abrir_externo(alvo: &std::ffi::OsStr) -> Result<(), String> {
    let abridor = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(abridor).arg(alvo).spawn().map(|_| ()).map_err(|e| format!("não abriu: {e}"))
}

/// O app veio de um pacote do sistema (.deb/.rpm): desinstalar pede a senha de administrador.
pub fn pelo_sistema(app: &App) -> bool {
    cfg!(all(unix, not(target_os = "macos"))) && executavel(app).is_some_and(|exe| !exe.starts_with(dirs::home_dir().unwrap_or_default()))
}

/// Tira o app do sistema e do registro do launcher. Os documentos e as
/// configurações da pessoa ficam onde estão.
pub fn desinstalar(app: &App) -> Result<(), String> {
    let exe = executavel(app).ok_or("não encontrei o aplicativo instalado")?;
    desinstalar_em(app, &exe)?;
    alterar_instalados(&pasta_dados(), |mapa| {
        mapa.remove(app.id);
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desinstalar_em(app: &App, exe: &Path) -> Result<(), String> {
    let local = dirs::home_dir().unwrap_or_default().join(".local");
    if !exe.starts_with(&local) {
        // .deb/.rpm: o gerenciador de pacotes remove, e o pkexec pede a senha na janela do sistema.
        let dpkg = Command::new("dpkg-query").args(["-W", app.id]).output().is_ok_and(|s| s.status.success());
        let args = if dpkg { ["dpkg", "-r", app.id] } else { ["rpm", "-e", app.id] };
        return rodar(Command::new("pkexec").args(args)).map_err(|_| format!("o pacote {} não foi removido (senha recusada ou cancelada)", app.id));
    }
    // Desfaz o instalar.sh.
    fs::remove_file(exe).map_err(|e| format!("não apagou {}: {e}", exe.display()))?;
    let share = local.join("share");
    let _ = fs::remove_file(share.join("applications").join(format!("{}.desktop", app.id)));
    let _ = fs::remove_file(share.join("mime/packages").join(format!("{}.xml", app.id)));
    for t in [16, 24, 32, 48, 64, 128, 256, 512] {
        let _ = fs::remove_file(share.join(format!("icons/hicolor/{t}x{t}/apps/{}.png", app.id)));
    }
    let _ = Command::new("update-mime-database").arg(share.join("mime")).status();
    let _ = Command::new("update-desktop-database").arg(share.join("applications")).status();
    Ok(())
}

#[cfg(windows)]
fn desinstalar_em(_app: &App, exe: &Path) -> Result<(), String> {
    // O desinstalador que o NSIS deixou na pasta, sem perguntas.
    let des = exe.parent().map(|p| p.join("desinstalar.exe")).filter(|d| d.exists()).ok_or("o desinstalador do aplicativo não foi encontrado")?;
    rodar(Command::new(des).arg("/S"))
}

#[cfg(target_os = "macos")]
fn desinstalar_em(_app: &App, exe: &Path) -> Result<(), String> {
    fs::remove_dir_all(exe).map_err(|e| format!("não apagou {}: {e}", exe.display()))
}

pub fn baixar_e_instalar(app: &'static App, rel: Release) -> impl Stream<Item = Evento> {
    crate::tarefas::fluxo(16, move |mut saida: Sender<Evento>| async move {
        let fim = executar(app, &rel, &mut saida).await.map(|_| rel.versao.clone());
        let _ = saida.send(Evento::Fim(fim)).await;
    })
}

async fn executar(app: &'static App, rel: &Release, saida: &mut Sender<Evento>) -> Result<(), String> {
    let pacote = rel.pacote.clone().ok_or("esta versão não tem pacote para este sistema")?;
    let esperado = soma_esperada(rel, &pacote.nome).await?;
    let arquivo = parcial(&pacote, &rel.versao);
    fs::create_dir_all(pasta_downloads()).map_err(|e| e.to_string())?;
    baixar(&pacote, &arquivo, saida).await?;
    let _ = saida.send(Evento::Instalando).await;
    let versao = rel.versao.clone();
    tokio::task::spawn_blocking(move || {
        let soma = sha256(&arquivo)?;
        if soma != esperado {
            let _ = fs::remove_file(&arquivo);
            return Err("o arquivo baixado não confere com o SHA256 da versão; baixe de novo".into());
        }
        instalar(app, &arquivo)?;
        let _ = fs::remove_file(&arquivo);
        alterar_instalados(&pasta_dados(), |mapa| {
            mapa.insert(app.id.into(), versao);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Troca o próprio launcher pela versão `rel` e devolve o executável novo, para reabrir.
/// Instalado pelo `.deb` (em `/usr/bin`), atualiza pelo `.deb` novo com `pkexec`.
pub async fn atualizar_launcher(rel: Release) -> Result<PathBuf, String> {
    let alvo = std::env::current_exe().map_err(|e| e.to_string())?;
    let pelo_deb = cfg!(target_os = "linux") && alvo.starts_with("/usr");
    let pacote = if pelo_deb { rel.arquivos.iter().find(|a| a.nome.ends_with("-linux-amd64.deb")).cloned() } else { rel.pacote.clone() };
    let pacote = pacote.ok_or("esta versão não tem pacote para este sistema")?;
    let esperado = soma_esperada(&rel, &pacote.nome).await?;
    let arquivo = parcial(&pacote, &rel.versao);
    fs::create_dir_all(pasta_downloads()).map_err(|e| e.to_string())?;
    // Ninguém ouve o progresso: são poucos MB e a tela só mostra "Atualizando…".
    let (mut saida, _) = pelicano::futures::channel::mpsc::channel(1);
    baixar(&pacote, &arquivo, &mut saida).await?;
    tokio::task::spawn_blocking(move || {
        if sha256(&arquivo)? != esperado {
            let _ = fs::remove_file(&arquivo);
            return Err("o arquivo baixado não confere com o SHA256 da versão; tente de novo".into());
        }
        let r = if pelo_deb { rodar(Command::new("pkexec").arg("dpkg").arg("-i").arg(&arquivo)) } else { trocar_launcher(&arquivo, &alvo) };
        let _ = fs::remove_file(&arquivo);
        r.map(|_| alvo)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(all(unix, not(target_os = "macos")))]
fn trocar_launcher(pacote: &Path, alvo: &Path) -> Result<(), String> {
    let tmp = pasta_downloads().join("featherlauncher-atualizar");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    rodar(Command::new("tar").arg("-xzf").arg(pacote).arg("-C").arg(&tmp))?;
    let novo = fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path().join("featherlauncher")).find(|p| p.exists()).ok_or("o pacote não tem o executável do launcher")?;
    let r = trocar_executavel(&novo, alvo);
    let _ = fs::remove_dir_all(&tmp);
    r
}

#[cfg(windows)]
fn trocar_launcher(pacote: &Path, alvo: &Path) -> Result<(), String> {
    // O .exe em uso não pode ser sobrescrito, mas pode ser renomeado; o .old sai na próxima abertura.
    let velho = alvo.with_extension("old");
    let _ = fs::remove_file(&velho);
    fs::rename(alvo, &velho).map_err(|e| format!("não deu para trocar {}: {e}", alvo.display()))?;
    fs::copy(pacote, alvo).map(|_| ()).map_err(|e| {
        let _ = fs::rename(&velho, alvo);
        e.to_string()
    })
}

#[cfg(target_os = "macos")]
fn trocar_launcher(_pacote: &Path, _alvo: &Path) -> Result<(), String> {
    Err("no macOS o launcher ainda não se atualiza sozinho".into())
}

/// Copia ao lado e renomeia por cima: o Linux deixa trocar o executável em uso, e
/// uma falha no meio não deixa o launcher pela metade.
#[cfg(unix)]
fn trocar_executavel(novo: &Path, alvo: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let sem_permissao = |e: std::io::Error| format!("sem permissão em {} ({e}); baixe o pacote novo na página do launcher", alvo.display());
    let ao_lado = alvo.with_extension("novo");
    fs::copy(novo, &ao_lado).map_err(sem_permissao)?;
    fs::set_permissions(&ao_lado, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    fs::rename(&ao_lado, alvo).map_err(sem_permissao)
}

/// Continua de onde parou quando já existe um `.parcial` (pedido com `Range`).
async fn baixar(pacote: &Arquivo, destino: &Path, saida: &mut Sender<Evento>) -> Result<(), String> {
    let ja = fs::metadata(destino).map(|m| m.len()).unwrap_or(0);
    if ja > 0 && ja == pacote.tamanho {
        return Ok(());
    }
    let mut pedido = cliente().get(&pacote.url);
    if ja > 0 {
        pedido = pedido.header(reqwest::header::RANGE, format!("bytes={ja}-"));
    }
    let mut resp = pedido.send().await.map_err(|e| format!("sem conexão: {e}"))?;
    // Tamanho desconhecido (release lida pelas páginas) e o .parcial já inteiro: o SHA256 confere.
    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        return Ok(());
    }
    if !resp.status().is_success() {
        return Err(format!("o download respondeu {}", resp.status()));
    }
    // Servidor que ignora o Range devolve 200 com o arquivo inteiro: recomeçar do zero.
    let continua = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut arq = fs::OpenOptions::new().create(true).write(true).append(continua).truncate(!continua).open(destino).map_err(|e| e.to_string())?;
    let mut baixado = if continua { ja } else { 0 };
    let total = if pacote.tamanho > 0 { pacote.tamanho } else { baixado + resp.content_length().unwrap_or(0) };
    let mut avisado = 0;
    while let Some(bloco) = resp.chunk().await.map_err(|e| format!("o download parou: {e}"))? {
        arq.write_all(&bloco).map_err(|e| e.to_string())?;
        baixado += bloco.len() as u64;
        if baixado - avisado >= 256 * 1024 {
            avisado = baixado;
            let _ = saida.send(Evento::Progresso { baixado, total }).await;
        }
    }
    let _ = saida.send(Evento::Progresso { baixado, total }).await;
    Ok(())
}

/// Sem soma publicada não instala: é a única garantia de que o arquivo chegou inteiro e é o da release.
async fn soma_esperada(rel: &Release, nome: &str) -> Result<String, String> {
    for lista in ["SHA256SUMS".to_string(), format!("{nome}.sha256")] {
        let Some(arq) = rel.arquivos.iter().find(|a| a.nome == lista) else { continue };
        let texto = cliente().get(&arq.url).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
        if let Some(soma) = procurar_soma(&texto, nome) {
            return Ok(soma);
        }
    }
    Err(format!("a versão não publica o SHA256 de {nome}; instalação recusada"))
}

/// Linha do `sha256sum`/`shasum`: "<soma>  [*]<caminho>", o caminho pode ter pasta.
fn procurar_soma(texto: &str, nome: &str) -> Option<String> {
    texto.lines().find_map(|linha| {
        let mut partes = linha.split_whitespace();
        let soma = partes.next()?;
        let arquivo = partes.next()?.trim_start_matches('*');
        (arquivo == nome || arquivo.ends_with(&format!("/{nome}"))).then(|| soma.to_lowercase())
    })
}

fn sha256(caminho: &Path) -> Result<String, String> {
    let mut arq = fs::File::open(caminho).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    std::io::copy(&mut arq, &mut h).map_err(|e| e.to_string())?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// No Windows, um programa de console (`reg`) aberto pelo launcher piscava uma janela preta.
pub fn sem_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn rodar(cmd: &mut Command) -> Result<(), String> {
    let st = sem_console(cmd).status().map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    st.success().then_some(()).ok_or_else(|| format!("{:?} terminou com {st}", cmd.get_program()))
}

fn instalar(app: &App, pacote: &Path) -> Result<(), String> {
    let tmp = pasta_downloads().join(format!("{}-instalar", app.id));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let r = instalar_em(app, pacote, &tmp);
    let _ = fs::remove_dir_all(&tmp);
    r
}

#[cfg(all(unix, not(target_os = "macos")))]
fn instalar_em(_app: &App, pacote: &Path, tmp: &Path) -> Result<(), String> {
    // O .tar.gz traz "<app>-<versão>/instalar.sh", o mesmo de quem instala à mão.
    rodar(Command::new("tar").arg("-xzf").arg(pacote).arg("-C").arg(tmp))?;
    let pasta = fs::read_dir(tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.join("instalar.sh").exists()).ok_or("o pacote não tem instalar.sh")?;
    rodar(Command::new("sh").arg(pasta.join("instalar.sh")))
}

#[cfg(windows)]
fn instalar_em(_app: &App, pacote: &Path, tmp: &Path) -> Result<(), String> {
    // Instalador NSIS por usuário (RequestExecutionLevel user); /S = sem perguntas.
    let exe = tmp.join("instalador.exe");
    fs::copy(pacote, &exe).map_err(|e| e.to_string())?;
    rodar(Command::new(&exe).arg("/S"))
}

#[cfg(target_os = "macos")]
fn instalar_em(app: &App, pacote: &Path, tmp: &Path) -> Result<(), String> {
    let montado = tmp.join("dmg");
    rodar(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-mountpoint"]).arg(&montado).arg(pacote))?;
    let destino = dirs::home_dir().unwrap_or_default().join("Applications");
    let nome = format!("{}.app", app.nome());
    let _ = fs::create_dir_all(&destino);
    let _ = fs::remove_dir_all(destino.join(&nome));
    let r = rodar(Command::new("ditto").arg(montado.join(&nome)).arg(destino.join(&nome)));
    let _ = rodar(Command::new("hdiutil").arg("detach").arg(&montado).arg("-quiet"));
    r
}

/// O atalho que o login do Linux abre sozinho (padrão XDG, vale para GNOME, KDE e os outros).
#[cfg(all(unix, not(target_os = "macos")))]
fn atalho_de_inicio() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("autostart/featherlauncher.desktop")
}

/// Ao entrar no computador, o launcher confere as atualizações em segundo plano (`--bandeja`)?
#[cfg(all(unix, not(target_os = "macos")))]
pub fn inicia_com_sistema() -> bool {
    atalho_de_inicio().exists()
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn iniciar_com_sistema(sim: bool) -> Result<(), String> {
    let atalho = atalho_de_inicio();
    if !sim {
        return fs::remove_file(&atalho).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e.to_string()) });
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    fs::create_dir_all(atalho.parent().unwrap()).map_err(|e| e.to_string())?;
    let entrada = format!("[Desktop Entry]\nType=Application\nName=FeatherWorks Launcher\nExec=\"{}\" --bandeja\nIcon=featherlauncher\nTerminal=false\nX-GNOME-Autostart-enabled=true\n", exe.display());
    fs::write(&atalho, entrada).map_err(|e| format!("não gravou {}: {e}", atalho.display()))
}

/// A chave que o Windows lê ao entrar o usuário; sem administrador.
#[cfg(windows)]
const CHAVE_INICIO: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
pub fn inicia_com_sistema() -> bool {
    sem_console(Command::new("reg").args(["query", CHAVE_INICIO, "/v", "FeatherWorksLauncher"])).output().is_ok_and(|s| s.status.success())
}

#[cfg(windows)]
pub fn iniciar_com_sistema(sim: bool) -> Result<(), String> {
    if !sim {
        // Sem a entrada, o `reg delete` falha; para quem pediu "não", tanto faz.
        let _ = sem_console(Command::new("reg").args(["delete", CHAVE_INICIO, "/v", "FeatherWorksLauncher", "/f"])).output();
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    rodar(Command::new("reg").args(["add", CHAVE_INICIO, "/v", "FeatherWorksLauncher", "/t", "REG_SZ", "/f", "/d"]).arg(format!("\"{}\" --bandeja", exe.display())))
}

#[cfg(target_os = "macos")]
pub fn inicia_com_sistema() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn iniciar_com_sistema(_sim: bool) -> Result<(), String> {
    Err("no macOS o launcher ainda não abre sozinho com o sistema".into())
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn abrir_com_o_sistema_liga_e_desliga() {
        // Só este teste mexe no XDG_CONFIG_HOME; os outros usam pastas próprias.
        let pasta = std::env::temp_dir().join(format!("featherlauncher-inicio-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &pasta);
        iniciar_com_sistema(true).unwrap();
        assert!(inicia_com_sistema());
        let exe = std::env::current_exe().unwrap();
        assert!(fs::read_to_string(atalho_de_inicio()).unwrap().contains(&format!("Exec=\"{}\" --bandeja", exe.display())));
        iniciar_com_sistema(false).unwrap();
        assert!(!inicia_com_sistema());
        iniciar_com_sistema(false).unwrap();
        let _ = fs::remove_dir_all(&pasta);
    }

    #[test]
    fn instalacoes_em_paralelo_nao_apagam_umas_as_outras() {
        let pasta = std::env::temp_dir().join(format!("featherlauncher-teste-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        std::thread::scope(|s| {
            for n in 0..8 {
                let pasta = &pasta;
                s.spawn(move || alterar_instalados(pasta, |m| { m.insert(format!("app{n}"), "1.0".into()); }).unwrap());
            }
        });
        assert_eq!(ler_instalados(&pasta).len(), 8);
        let _ = fs::remove_dir_all(&pasta);
    }

    #[cfg(unix)]
    #[test]
    fn troca_o_executavel_por_cima() {
        let pasta = std::env::temp_dir().join(format!("featherlauncher-troca-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).unwrap();
        let (novo, alvo) = (pasta.join("baixado"), pasta.join("featherlauncher"));
        fs::write(&novo, "versão nova").unwrap();
        fs::write(&alvo, "versão velha").unwrap();
        trocar_executavel(&novo, &alvo).unwrap();
        assert_eq!(fs::read_to_string(&alvo).unwrap(), "versão nova");
        assert!(!alvo.with_extension("novo").exists());
        let _ = fs::remove_dir_all(&pasta);
    }

    #[test]
    fn limpa_a_versao_do_pacote() {
        assert_eq!(limpar_versao("0.1.3-1").as_deref(), Some("0.1.3"));
        assert_eq!(limpar_versao("1:0.2.0~rc1").as_deref(), Some("0.2.0"));
        assert_eq!(limpar_versao("desconhecida"), None);
    }

    #[test]
    fn acha_a_soma_no_sha256sums_e_no_shasum() {
        let sums = "947c  FeatherSheet-windows-x64.exe\n1F25  feathersheet-linux-x86_64.tar.gz\n";
        assert_eq!(procurar_soma(sums, "feathersheet-linux-x86_64.tar.gz").as_deref(), Some("1f25"));
        assert_eq!(procurar_soma("abcd  dist-mac/X.dmg\n", "X.dmg").as_deref(), Some("abcd"));
        assert_eq!(procurar_soma("abcd *X.dmg", "X.dmg").as_deref(), Some("abcd"));
        assert_eq!(procurar_soma(sums, "outro.dmg"), None);
    }
}
