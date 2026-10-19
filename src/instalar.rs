//! Baixar, conferir o SHA256, instalar e abrir. Tudo na pasta do usuário,
//! sem pedir senha de administrador.
use crate::releases::{cliente, App, Arquivo, Release};
use iced::futures::channel::mpsc::Sender;
use iced::futures::{SinkExt, Stream};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// O arquivo baixado pela metade fica aqui; é dele que a pausa continua.
pub fn parcial(pacote: &Arquivo) -> PathBuf {
    pasta_downloads().join(format!("{}.parcial", pacote.nome))
}

/// Versões instaladas pelo launcher: `{"feathersheet": "0.1.3"}`.
pub fn instalados() -> HashMap<String, String> {
    fs::read(pasta_dados().join("instalados.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn registrar(id: &str, versao: &str) -> Result<(), String> {
    let mut mapa = instalados();
    mapa.insert(id.into(), versao.into());
    fs::create_dir_all(pasta_dados()).map_err(|e| e.to_string())?;
    fs::write(pasta_dados().join("instalados.json"), serde_json::to_vec_pretty(&mapa).unwrap()).map_err(|e| e.to_string())
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
    let saida = |cmd: &mut Command| cmd.output().ok().filter(|s| s.status.success()).map(|s| String::from_utf8_lossy(&s.stdout).trim().to_string()).filter(|s| !s.is_empty());
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
    cmd.spawn().map(|_| ()).map_err(|e| format!("não abriu: {e}"))
}

/// Abre a pasta onde o app está no gerenciador de arquivos do sistema.
pub fn mostrar_pasta(app: &App) -> Result<(), String> {
    let exe = executavel(app).ok_or("não encontrei o aplicativo instalado")?;
    let pasta = exe.parent().unwrap_or(&exe);
    let abridor = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(abridor).arg(pasta).spawn().map(|_| ()).map_err(|e| format!("não abriu a pasta: {e}"))
}

/// Tira o app do sistema e do registro do launcher. Os documentos e as
/// configurações da pessoa ficam onde estão.
pub fn desinstalar(app: &App) -> Result<(), String> {
    let exe = executavel(app).ok_or("não encontrei o aplicativo instalado")?;
    desinstalar_em(app, &exe)?;
    let mut mapa = instalados();
    if mapa.remove(app.id).is_some() {
        fs::write(pasta_dados().join("instalados.json"), serde_json::to_vec_pretty(&mapa).unwrap()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desinstalar_em(app: &App, exe: &Path) -> Result<(), String> {
    // Desfaz o instalar.sh; o que veio de .deb/.rpm é do gerenciador de pacotes.
    let local = dirs::home_dir().unwrap_or_default().join(".local");
    if !exe.starts_with(&local) {
        return Err(format!("{} foi instalado pelo sistema; desinstale pelo gerenciador de pacotes", app.nome()));
    }
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
    iced::stream::channel(16, move |mut saida: Sender<Evento>| async move {
        let fim = executar(app, &rel, &mut saida).await.map(|_| rel.versao.clone());
        let _ = saida.send(Evento::Fim(fim)).await;
    })
}

async fn executar(app: &'static App, rel: &Release, saida: &mut Sender<Evento>) -> Result<(), String> {
    let pacote = rel.pacote.clone().ok_or("esta versão não tem pacote para este sistema")?;
    let esperado = soma_esperada(rel, &pacote.nome).await?;
    let arquivo = parcial(&pacote);
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
        registrar(app.id, &versao)
    })
    .await
    .map_err(|e| e.to_string())?
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
    if !resp.status().is_success() {
        return Err(format!("o download respondeu {}", resp.status()));
    }
    // Servidor que ignora o Range devolve 200 com o arquivo inteiro: recomeçar do zero.
    let continua = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut arq = fs::OpenOptions::new().create(true).write(true).append(continua).truncate(!continua).open(destino).map_err(|e| e.to_string())?;
    let mut baixado = if continua { ja } else { 0 };
    let mut avisado = 0;
    while let Some(bloco) = resp.chunk().await.map_err(|e| format!("o download parou: {e}"))? {
        arq.write_all(&bloco).map_err(|e| e.to_string())?;
        baixado += bloco.len() as u64;
        if baixado - avisado >= 256 * 1024 {
            avisado = baixado;
            let _ = saida.send(Evento::Progresso { baixado, total: pacote.tamanho }).await;
        }
    }
    let _ = saida.send(Evento::Progresso { baixado, total: pacote.tamanho }).await;
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

fn rodar(cmd: &mut Command) -> Result<(), String> {
    let st = cmd.status().map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
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

#[cfg(test)]
mod testes {
    use super::*;

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
