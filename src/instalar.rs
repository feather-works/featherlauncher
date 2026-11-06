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
    if cfg!(windows) { "%ProgramFiles%\\FeatherWorks" } else if cfg!(target_os = "macos") { "~/Applications" } else { "~/.local" }
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
        // Program Files\FeatherWorks (instalador para todos); %LOCALAPPDATA%\Programs é o das
        // versões antigas, por usuário, até cada app passar para a pasta da suíte.
        let exe = format!("{}.exe", app.id);
        let local = dirs::data_local_dir().unwrap_or_default();
        let todos = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_default();
        vec![todos.join("FeatherWorks").join(app.nome()).join(&exe), local.join("Programs").join(app.nome()).join(&exe)]
    } else if cfg!(target_os = "macos") {
        vec![casa.join("Applications").join(format!("{}.app", app.nome())), PathBuf::from(format!("/Applications/{}.app", app.nome()))]
    } else {
        vec![casa.join(".local/bin").join(app.id), Path::new("/usr/bin").join(app.id), Path::new("/snap/bin").join(nome_snap(app))]
    };
    candidatos.into_iter().find(|c| c.exists())
}

/// De onde veio uma cópia do app. No Linux o mesmo app pode estar ao mesmo tempo pelo launcher,
/// pelo .deb, pela Snap Store e pelo Flatpak: o launcher usa e atualiza a que já existe, pelo
/// caminho dela, em vez de instalar mais uma em `~/.local/bin` (era assim que um .deb antigo
/// ficava esquecido ao lado da cópia nova).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origem {
    Launcher,
    Pacote,
    Snap,
    Flatpak,
}

impl Origem {
    pub fn nome(self) -> &'static str {
        match self {
            Origem::Launcher => "launcher",
            Origem::Pacote => "pacote do sistema",
            Origem::Snap => "Snap Store",
            Origem::Flatpak => "Flatpak",
        }
    }
}

/// O FeatherNotes foi publicado na Snap Store como `feather-notes`; os outros com o próprio id.
fn nome_snap(app: &App) -> &'static str {
    if app.id == "feathernotes" { "feather-notes" } else { app.id }
}

fn id_flatpak(app: &App) -> String {
    format!("io.github.feather_works.{}", app.nome())
}

/// `--user` ou `--system`, conforme onde o Flatpak do app está; `None` sem Flatpak.
fn escopo_flatpak(app: &App) -> Option<&'static str> {
    let id = id_flatpak(app);
    let casa = dirs::data_dir().unwrap_or_default().join("flatpak/app").join(&id);
    if casa.exists() {
        Some("--user")
    } else {
        Path::new("/var/lib/flatpak/app").join(&id).exists().then_some("--system")
    }
}

/// As cópias instaladas, da que o launcher usa (a primeira) para a última. Só olha arquivos:
/// roda a cada "Verificar", para os sete apps.
pub fn copias(app: &App) -> Vec<Origem> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return executavel(app).map(|_| vec![Origem::Launcher]).unwrap_or_default();
    }
    let casa = dirs::home_dir().unwrap_or_default();
    let mut v = Vec::new();
    if casa.join(".local/bin").join(app.id).exists() {
        v.push(Origem::Launcher);
    }
    if Path::new("/usr/bin").join(app.id).exists() {
        v.push(Origem::Pacote);
    }
    if Path::new("/snap/bin").join(nome_snap(app)).exists() {
        v.push(Origem::Snap);
    }
    if escopo_flatpak(app).is_some() {
        v.push(Origem::Flatpak);
    }
    v
}

/// A cópia que o launcher instalou; no Linux só a de `~/.local/bin` (a de `/usr/bin` é do .deb).
fn copia_do_launcher(app: &App) -> Option<PathBuf> {
    if cfg!(all(unix, not(target_os = "macos"))) {
        Some(dirs::home_dir()?.join(".local/bin").join(app.id)).filter(|p| p.exists())
    } else {
        executavel(app)
    }
}

fn saida(cmd: &mut Command) -> Option<String> {
    cmd.output().ok().filter(|s| s.status.success()).map(|s| String::from_utf8_lossy(&s.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// A versão da cópia que veio por fora do launcher, perguntada a quem a instalou.
pub fn versao_de(app: &App, origem: Origem) -> Option<String> {
    match origem {
        // No Linux a do launcher está no registro: o dpkg responderia a versão do .deb, não a dela.
        Origem::Launcher if cfg!(all(unix, not(target_os = "macos"))) => None,
        Origem::Launcher => executavel(app).and_then(|exe| versao_externa(app, &exe)),
        Origem::Pacote => {
            let v = saida(Command::new("dpkg-query").args(["-W", "-f=${Version}", app.id])).or_else(|| saida(Command::new("rpm").args(["-q", "--qf", "%{VERSION}", app.id])))?;
            limpar_versao(&v)
        }
        Origem::Snap => versao_snap(&saida(Command::new("snap").args(["list", nome_snap(app)]))?, nome_snap(app)),
        Origem::Flatpak => versao_flatpak(&saida(Command::new("flatpak").args(["list", "--app", "--columns=application,version"]))?, &id_flatpak(app)),
    }
}

/// `snap list <nome>`: cabeçalho e uma linha "nome  versão  rev  canal  publicador  notas".
fn versao_snap(lista: &str, nome: &str) -> Option<String> {
    let linha = lista.lines().find(|l| l.split_whitespace().next() == Some(nome))?;
    limpar_versao(linha.split_whitespace().nth(1)?)
}

/// `flatpak list --app --columns=application,version`: "id<TAB>versão" por linha.
fn versao_flatpak(lista: &str, id: &str) -> Option<String> {
    let linha = lista.lines().find(|l| l.split_whitespace().next() == Some(id))?;
    limpar_versao(linha.split_whitespace().nth(1)?)
}

/// Atualiza a cópia que não é do launcher pelo caminho dela e devolve a versão que ficou.
/// O .deb vem da mesma release (com o SHA256 conferido); Snap e Flatpak atualizam pela loja
/// deles, que às vezes ainda não tem a versão do GitHub.
pub async fn atualizar_por_fora(app: &'static App, origem: Origem, rel: Release) -> Result<String, String> {
    match origem {
        Origem::Launcher => Err("a cópia do launcher se atualiza pelo pacote da release".into()),
        Origem::Pacote => {
            let dpkg = Command::new("dpkg-query").args(["-W", app.id]).output().is_ok_and(|s| s.status.success());
            if !dpkg {
                return Err(format!("{} foi instalado pelo rpm: atualize pelo gerenciador de pacotes do sistema", app.nome()));
            }
            let pacote = rel.arquivos.iter().find(|a| a.nome.ends_with("-linux-amd64.deb")).cloned().ok_or("esta versão não tem .deb")?;
            let esperado = soma_esperada(&rel, &pacote.nome).await?;
            let arquivo = parcial(&pacote, &rel.versao);
            fs::create_dir_all(pasta_downloads()).map_err(|e| e.to_string())?;
            let (mut progresso, _) = pelicano::futures::channel::mpsc::channel(1);
            baixar(&pacote, &arquivo, &mut progresso).await?;
            let versao = rel.versao.clone();
            tokio::task::spawn_blocking(move || {
                let r = if sha256(&arquivo)? != esperado {
                    Err("o arquivo baixado não confere com o SHA256 da versão; tente de novo".to_string())
                } else {
                    rodar(Command::new("pkexec").arg("dpkg").arg("-i").arg(&arquivo)).map_err(|_| format!("{} não foi atualizado (senha recusada ou cancelada)", app.nome()))
                };
                let _ = fs::remove_file(&arquivo);
                r.map(|_| versao)
            })
            .await
            .map_err(|e| e.to_string())?
        }
        Origem::Snap | Origem::Flatpak => {
            let antes = versao_de(app, origem);
            tokio::task::spawn_blocking(move || {
                let r = if origem == Origem::Snap {
                    rodar(Command::new("pkexec").args(["snap", "refresh", nome_snap(app)]))
                } else {
                    let escopo = escopo_flatpak(app).unwrap_or("--user");
                    rodar(Command::new("flatpak").args(["update", "-y", "--noninteractive", escopo, &id_flatpak(app)]))
                };
                r.map_err(|_| format!("{} não foi atualizado pela {}", app.nome(), origem.nome()))?;
                let depois = versao_de(app, origem).ok_or("não deu para ler a versão depois de atualizar")?;
                if Some(&depois) == antes.as_ref() && depois != rel.versao {
                    return Err(format!("a {} ainda não tem a {}; quando chegar, ela atualiza sozinha", origem.nome(), rel.versao));
                }
                Ok(depois)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

/// Remove uma cópia pelo caminho dela. O launcher também tira o app do registro.
pub fn desinstalar_copia(app: &App, origem: Origem) -> Result<(), String> {
    let r = match origem {
        Origem::Launcher => copia_do_launcher(app).ok_or("não encontrei o aplicativo instalado".to_string()).and_then(|exe| desinstalar_em(app, &exe)),
        Origem::Pacote => desinstalar_em(app, &Path::new("/usr/bin").join(app.id)),
        Origem::Snap => rodar(Command::new("pkexec").args(["snap", "remove", nome_snap(app)])).map_err(|_| format!("a cópia da Snap Store não foi removida (senha recusada ou cancelada)")),
        Origem::Flatpak => {
            let escopo = escopo_flatpak(app).unwrap_or("--user");
            rodar(Command::new("flatpak").args(["uninstall", "-y", "--noninteractive", escopo, &id_flatpak(app)])).map_err(|_| "a cópia do Flatpak não foi removida".to_string())
        }
    };
    r?;
    if origem == Origem::Launcher {
        alterar_instalados(&pasta_dados(), |mapa| {
            mapa.remove(app.id);
        })?;
    }
    Ok(())
}

/// A versão de um app que o launcher não instalou, perguntada a quem instalou
/// (.deb/.rpm, instalador do Windows, Info.plist do .app). Rodar o executável
/// com `--version` não serve: ele abriria a janela.
#[cfg_attr(windows, allow(unused_variables))]
pub fn versao_externa(app: &App, exe: &Path) -> Option<String> {
    #[cfg(windows)]
    let v = {
        let chave = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", app.nome());
        registro::ler_todos(&chave, "DisplayVersion").or_else(|| registro::ler(&chave, "DisplayVersion"))?
    };
    #[cfg(not(windows))]
    let v = {
        let saida = |cmd: &mut Command| cmd.output().ok().filter(|s| s.status.success()).map(|s| String::from_utf8_lossy(&s.stdout).trim().to_string()).filter(|s| !s.is_empty());
        if cfg!(target_os = "macos") {
            saida(Command::new("defaults").arg("read").arg(exe.join("Contents/Info")).arg("CFBundleShortVersionString"))?
        } else {
            saida(Command::new("dpkg-query").args(["-W", "-f=${Version}", app.id])).or_else(|| saida(Command::new("rpm").args(["-q", "--qf", "%{VERSION}", app.id])))?
        }
    };
    limpar_versao(&v)
}

/// "0.1.3-1" (revisão do pacote) e "1:0.1.3" (época) → "0.1.3".
fn limpar_versao(v: &str) -> Option<String> {
    let v = v.rsplit(':').next()?.split(['-', '~', '+']).next()?;
    v.starts_with(|c: char| c.is_ascii_digit()).then(|| v.to_string())
}

pub fn abrir(app: &App) -> Result<(), String> {
    if copias(app).first() == Some(&Origem::Flatpak) {
        return Command::new("flatpak").args(["run", &id_flatpak(app)]).spawn().map(|_| ()).map_err(|e| format!("não abriu: {e}"));
    }
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
    if let (Some(Origem::Flatpak), Some(escopo)) = (copias(app).first(), escopo_flatpak(app)) {
        let base = if escopo == "--user" { dirs::data_dir().unwrap_or_default().join("flatpak/app") } else { PathBuf::from("/var/lib/flatpak/app") };
        return abrir_externo(base.join(id_flatpak(app)).as_os_str());
    }
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
    matches!(copias(app).first(), Some(Origem::Pacote | Origem::Snap))
}

/// Tira o app do sistema e do registro do launcher. Os documentos e as
/// configurações da pessoa ficam onde estão.
pub fn desinstalar(app: &App) -> Result<(), String> {
    let principal = *copias(app).first().ok_or("não encontrei o aplicativo instalado")?;
    desinstalar_copia(app, principal)
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
    // O desinstalador que o NSIS deixou na pasta, sem perguntas (em Program Files o Windows pede administrador).
    let des = exe.parent().map(|p| p.join("desinstalar.exe")).filter(|d| d.exists()).ok_or("o desinstalador do aplicativo não foi encontrado")?;
    abrir_e_esperar(&des, "/S")
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
    // Instalado em Program Files (pelo instalador), o .exe só muda com administrador: atualiza
    // pelo instalador novo, que pede o UAC e troca o .exe em uso.
    let pelo_instalador = cfg!(windows) && std::env::var_os("ProgramFiles").is_some_and(|p| alvo.starts_with(p));
    let pacote = if pelo_deb {
        rel.arquivos.iter().find(|a| a.nome.ends_with("-linux-amd64.deb")).cloned()
    } else if pelo_instalador {
        rel.arquivos.iter().find(|a| a.nome.ends_with("-windows-x64-instalador.exe")).cloned()
    } else {
        rel.pacote.clone()
    };
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
        let r = if pelo_deb {
            rodar(Command::new("pkexec").arg("dpkg").arg("-i").arg(&arquivo))
        } else if pelo_instalador {
            atualizar_pelo_instalador(&arquivo)
        } else {
            trocar_launcher(&arquivo, &alvo)
        };
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

#[cfg(windows)]
fn atualizar_pelo_instalador(pacote: &Path) -> Result<(), String> {
    let tmp = pasta_downloads().join("featherlauncher-atualizar");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let r = rodar_instalador(pacote, &tmp);
    let _ = fs::remove_dir_all(&tmp);
    r
}

#[cfg(not(windows))]
fn atualizar_pelo_instalador(_pacote: &Path) -> Result<(), String> {
    unreachable!("só no Windows")
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
    rodar_instalador(pacote, tmp)
}

/// Instalador NSIS; /S = sem perguntas, como no "Instalar" de um clique. Com o nome da release
/// ("0.2.6-X.exe.parcial" → "X.exe") e sem esconder o processo: programa baixado rodado
/// escondido é o que o Defender acusa. O de Program Files pede administrador (UAC).
#[cfg(windows)]
fn rodar_instalador(pacote: &Path, tmp: &Path) -> Result<(), String> {
    let nome = pacote.file_name().and_then(|n| n.to_str()).unwrap_or("instalador.exe").trim_end_matches(".parcial");
    let exe = tmp.join(nome.split_once('-').map_or(nome, |(_, n)| n));
    fs::copy(pacote, &exe).map_err(|e| e.to_string())?;
    abrir_e_esperar(&exe, "/S")
}

/// Abre `exe` pelo ShellExecuteEx e espera terminar. Diferente do `Command`, ele respeita o
/// manifesto do instalador que pede administrador: o Windows mostra o UAC em vez de recusar.
#[cfg(windows)]
fn abrir_e_esperar(exe: &Path, args: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let largo = |s: &std::ffi::OsStr| -> Vec<u16> { std::os::windows::ffi::OsStrExt::encode_wide(s).chain([0]).collect() };
    let (arquivo, parametros) = (largo(exe.as_os_str()), largo(std::ffi::OsStr::new(args)));
    // SAFETY: estrutura zerada com o tamanho certo, textos terminados em zero que vivem até o fim.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpFile = arquivo.as_ptr();
    info.lpParameters = parametros.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let e = std::io::Error::last_os_error();
        // 1223 = ERROR_CANCELLED: a pessoa disse "Não" no pedido de administrador.
        return Err(if e.raw_os_error() == Some(1223) { "cancelado: o Windows pediu permissão de administrador e ela foi negada".into() } else { format!("não abriu {}: {e}", exe.display()) });
    }
    let mut codigo = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut codigo);
        CloseHandle(info.hProcess);
    }
    (codigo == 0).then_some(()).ok_or_else(|| format!("{} terminou com o código {codigo}", exe.display()))
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

/// Entrada de antes da bandeja (abria a janela inteira no login): regrava com o `--bandeja`.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn atualizar_inicio() {
    if fs::read_to_string(atalho_de_inicio()).is_ok_and(|e| !e.contains("--bandeja")) {
        let _ = iniciar_com_sistema(true);
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn atualizar_inicio() {}

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

/// A chave que o Windows lê ao entrar o usuário (em HKCU); sem administrador.
#[cfg(windows)]
const CHAVE_INICIO: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
pub fn inicia_com_sistema() -> bool {
    registro::ler(CHAVE_INICIO, "FeatherWorksLauncher").is_some()
}

#[cfg(windows)]
pub fn iniciar_com_sistema(sim: bool) -> Result<(), String> {
    if !sim {
        // Sem a entrada, apagar falha; para quem pediu "não", tanto faz.
        registro::apagar(CHAVE_INICIO, "FeatherWorksLauncher");
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    registro::gravar(CHAVE_INICIO, "FeatherWorksLauncher", &format!("\"{}\" --bandeja", exe.display()))
}

/// Valores de texto em HKCU pela API do Windows: o `reg.exe` escondido que havia antes
/// era um dos motivos de o Defender acusar o launcher.
#[cfg(windows)]
mod registro {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    fn largo(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// Em HKLM: o que os instaladores para todos os usuários (Program Files) gravam.
    pub fn ler_todos(chave: &str, nome: &str) -> Option<String> {
        ler_em(windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE, chave, nome)
    }

    pub fn ler(chave: &str, nome: &str) -> Option<String> {
        ler_em(HKEY_CURRENT_USER, chave, nome)
    }

    fn ler_em(raiz: windows_sys::Win32::System::Registry::HKEY, chave: &str, nome: &str) -> Option<String> {
        let (chave, nome) = (largo(chave), largo(nome));
        let mut buf = [0u16; 1024];
        let mut tam = std::mem::size_of_val(&buf) as u32;
        let r = unsafe { RegGetValueW(raiz, chave.as_ptr(), nome.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut tam) };
        (r == 0).then(|| String::from_utf16_lossy(&buf[..(tam as usize / 2).saturating_sub(1)]))
    }

    pub fn gravar(chave: &str, nome: &str, valor: &str) -> Result<(), String> {
        let (chave, nome, valor) = (largo(chave), largo(nome), largo(valor));
        let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, chave.as_ptr(), nome.as_ptr(), REG_SZ, valor.as_ptr().cast(), (valor.len() * 2) as u32) };
        (r == 0).then_some(()).ok_or_else(|| format!("não gravou no registro (erro {r})"))
    }

    pub fn apagar(chave: &str, nome: &str) {
        let (chave, nome) = (largo(chave), largo(nome));
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, chave.as_ptr(), nome.as_ptr()) };
    }
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
    fn le_a_versao_do_snap_e_do_flatpak() {
        let snap = "Name          Version  Rev  Tracking       Publisher    Notes\nfeathersheet  0.2.9    3    latest/stable  marceloagon  -";
        assert_eq!(versao_snap(snap, "feathersheet").as_deref(), Some("0.2.9"));
        assert_eq!(versao_snap(snap, "featherpdf"), None);
        let flatpak = "io.github.feather_works.FeatherPDF\t0.1.4\nio.github.feather_works.FeatherSheet\t0.2.8";
        assert_eq!(versao_flatpak(flatpak, "io.github.feather_works.FeatherSheet").as_deref(), Some("0.2.8"));
        assert_eq!(versao_flatpak(flatpak, "io.github.feather_works.FeatherNotes"), None);
        assert_eq!(nome_snap(&crate::releases::APPS[6]), "feather-notes");
        assert_eq!(id_flatpak(&crate::releases::APPS[0]), "io.github.feather_works.FeatherSheet");
    }

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
