//! O catálogo da suíte e a consulta às releases do GitHub.

pub const ORG: &str = "feather-works";
/// Começo da mensagem de cota estourada; a tela a reconhece por ele.
pub const LIMITE: &str = "o GitHub limitou as consultas";

pub struct App {
    pub id: &'static str,
    /// Segunda metade do nome ("Sheet" em "FeatherSheet"), pintada na cor do app.
    pub sufixo: &'static str,
    pub descricao: &'static str,
    pub cor: &'static str,
}

impl App {
    pub fn nome(&self) -> String {
        format!("Feather{}", self.sufixo)
    }
}

pub const APPS: [App; 6] = [
    App { id: "feathersheet", sufixo: "Sheet", descricao: "Planilhas e dados", cor: "#287846" },
    App { id: "featherslide", sufixo: "Slide", descricao: "Apresentações", cor: "#a87f0c" },
    App { id: "featherdocs", sufixo: "Docs", descricao: "Documentos de texto", cor: "#236ac7" },
    App { id: "featherpdf", sufixo: "PDF", descricao: "Leitura e organização de PDF", cor: "#c5352b" },
    App { id: "feathercad", sufixo: "CAD", descricao: "Desenho técnico 2D e 3D", cor: "#0f7184" },
    App { id: "feathercode", sufixo: "Code", descricao: "Editor de código", cor: "#7048b8" },
];

#[derive(Debug, Clone)]
pub struct Arquivo {
    pub nome: String,
    pub url: String,
    pub tamanho: u64,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub versao: String,
    pub data: String,
    /// O pacote deste sistema; `None` quando a release não tem um para ele.
    pub pacote: Option<Arquivo>,
    pub arquivos: Vec<Arquivo>,
}

/// Fim do nome do pacote de cada sistema, como o `pacote/gerar.sh` e o
/// `macos.yml` do FeatherSheet publicam.
pub fn sufixo_do_sistema() -> &'static str {
    if cfg!(windows) { "-windows-x64.exe" } else if cfg!(target_os = "macos") { ".dmg" } else { "-linux-x86_64.tar.gz" }
}

pub fn nome_do_sistema() -> &'static str {
    if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" }
}

/// Um cliente só para o app inteiro: reaproveita as conexões e a configuração do TLS.
pub fn cliente() -> &'static reqwest::Client {
    static CLIENTE: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENTE.get_or_init(|| reqwest::Client::builder().user_agent(concat!("featherlauncher/", env!("CARGO_PKG_VERSION"))).build().expect("cliente HTTP"))
}

/// `Ok(None)` = o app ainda não tem release (ou nem repositório).
///
/// Sem login o GitHub dá 60 consultas por hora; a resposta fica guardada com o
/// ETag e a próxima pergunta só "mudou?" — o 304 não gasta a cota. Com a cota
/// estourada, vale o que estava guardado.
pub async fn ultima(id: &'static str) -> Result<Option<Release>, String> {
    let guardado = ler_guardado(id);
    let url = format!("https://api.github.com/repos/{ORG}/{id}/releases/latest");
    let mut pedido = cliente().get(url);
    if let Some(etag) = guardado.as_ref().and_then(|g| g["etag"].as_str()) {
        pedido = pedido.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let resp = pedido.send().await.map_err(|e| format!("sem conexão: {e}"))?;
    let status = resp.status();
    let do_guardado = |g: &serde_json::Value| g["corpo"].as_str().map(|c| serde_json::from_str(c).map(|j| ler_release(&j)).map_err(|e| e.to_string())).transpose();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        if let Some(g) = &guardado {
            return do_guardado(g);
        }
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        guardar(id, None, None);
        return Ok(None);
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Sem cota na API: as páginas do github.com não contam.
        if let Ok(r) = pelas_paginas(id).await {
            return Ok(r);
        }
        if let Some(g) = &guardado {
            return do_guardado(g);
        }
        let minutos = resp
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
            .and_then(|fim| fim.checked_sub(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()))
            .map_or(String::new(), |s| format!(" em {} min", s.div_ceil(60)));
        return Err(format!("{LIMITE} por agora; tente de novo{minutos}"));
    }
    if !status.is_success() {
        return Err(format!("GitHub respondeu {status}"));
    }
    let etag = resp.headers().get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_string);
    let corpo = resp.text().await.map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&corpo).map_err(|e| e.to_string())?;
    guardar(id, etag.as_deref(), Some(&corpo));
    Ok(Some(ler_release(&json)))
}

/// O caminho sem cota: o feed `releases.atom` diz a tag mais nova e o `SHA256SUMS`
/// dela lista os arquivos. (O redirecionamento de `releases/latest` fica minutos
/// em cache depois de uma release nova; o feed não.) Não traz tamanho nem data;
/// o tamanho chega com o download.
async fn pelas_paginas(id: &str) -> Result<Option<Release>, String> {
    let resp = cliente().get(format!("https://github.com/{ORG}/{id}/releases.atom")).send().await.map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let feed = resp.text().await.map_err(|e| e.to_string())?;
    let Some((tag, quando)) = entrada_do_feed(&feed) else { return Ok(None) };
    let base = format!("https://github.com/{ORG}/{id}/releases/download/{tag}/");
    let resp = cliente().get(format!("{base}SHA256SUMS")).send().await.map_err(|e| e.to_string())?;
    // O feed lista também tag sem release (o FeatherDocs tem a v0.0.1 assim): sem
    // SHA256SUMS não há o que instalar, então conta como "sem release".
    // shortcut: só olha a entrada mais nova; uma tag solta mais nova esconde uma release antiga.
    if !resp.status().is_success() {
        return Ok(None);
    }
    let somas = resp.text().await.map_err(|e| e.to_string())?;
    let mut r = release_das_somas(&tag, &base, &somas);
    r.data = data_br(&quando);
    Ok(Some(r))
}

/// A primeira entrada do feed é a mais nova: a tag e quando saiu.
fn entrada_do_feed(feed: &str) -> Option<(String, String)> {
    let entrada = &feed[feed.find("<entry>")?..];
    let depois = |marca: &str| entrada.find(marca).map(|i| &entrada[i + marca.len()..]);
    let tag = depois("/releases/tag/")?;
    let quando = depois("<updated>").unwrap_or_default();
    Some((tag[..tag.find(['"', '<'])?].to_string(), quando[..quando.find('<').unwrap_or(0)].to_string()))
}

fn release_das_somas(tag: &str, base: &str, somas: &str) -> Release {
    let mut arquivos: Vec<Arquivo> = somas
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(|n| n.trim_start_matches('*').rsplit('/').next().unwrap_or(n).to_string())
        .map(|nome| Arquivo { url: format!("{base}{nome}"), nome, tamanho: 0 })
        .collect();
    if !somas.is_empty() {
        arquivos.push(Arquivo { nome: "SHA256SUMS".into(), url: format!("{base}SHA256SUMS"), tamanho: 0 });
    }
    let pacote = arquivos.iter().find(|a| a.nome.ends_with(sufixo_do_sistema())).cloned();
    Release { versao: tag.trim_start_matches('v').to_string(), data: "—".into(), pacote, arquivos }
}

fn arquivo_guardado(id: &str) -> std::path::PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("featherworks/releases").join(format!("{id}.json"))
}

fn ler_guardado(id: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(arquivo_guardado(id)).ok()?).ok()
}

/// `corpo` nulo = sem release; guardado também, para valer quando a cota estourar.
fn guardar(id: &str, etag: Option<&str>, corpo: Option<&str>) {
    let caminho = arquivo_guardado(id);
    let _ = std::fs::create_dir_all(caminho.parent().unwrap());
    let _ = std::fs::write(caminho, serde_json::json!({ "etag": etag, "corpo": corpo }).to_string());
}

fn ler_release(json: &serde_json::Value) -> Release {
    let texto = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let arquivos: Vec<Arquivo> = json["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| Arquivo { nome: texto(&a["name"]), url: texto(&a["browser_download_url"]), tamanho: a["size"].as_u64().unwrap_or(0) })
        .collect();
    let pacote = arquivos.iter().find(|a| a.nome.ends_with(sufixo_do_sistema())).cloned();
    Release {
        versao: texto(&json["tag_name"]).trim_start_matches('v').to_string(),
        data: data_br(&texto(&json["published_at"])),
        pacote,
        arquivos,
    }
}

/// "2026-10-20T00:23:34Z" (UTC) → "19/10/2026" no fuso desta máquina.
fn data_br(iso: &str) -> String {
    data_no_fuso(iso, fuso_local_min())
}

fn data_no_fuso(iso: &str, fuso_min: i64) -> String {
    let n = |a: usize, b: usize| iso.get(a..b)?.parse::<i64>().ok();
    let (Some(ano), Some(mes), Some(dia)) = (n(0, 4), n(5, 7), n(8, 10)) else { return "—".into() };
    let minutos = dias_desde_1970(ano, mes, dia) * 1440 + n(11, 13).unwrap_or(0) * 60 + n(14, 16).unwrap_or(0) + fuso_min;
    let (a, m, d) = data_civil(minutos.div_euclid(1440));
    format!("{d:02}/{m:02}/{a}")
}

/// O fuso desta máquina em minutos, pelo `date +%z` ("-0300" → -180).
// shortcut: no Windows não há `date +%z` e a data fica em UTC; trocar por GetTimeZoneInformation se pesar.
fn fuso_local_min() -> i64 {
    static FUSO: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *FUSO.get_or_init(|| {
        let saida = std::process::Command::new("date").arg("+%z").output().ok();
        let z = saida.map(|s| String::from_utf8_lossy(&s.stdout).trim().to_string()).unwrap_or_default();
        let sinal = if z.starts_with('-') { -1 } else { 1 };
        let n = |a: usize, b: usize| z.get(a..b).and_then(|v| v.parse::<i64>().ok());
        n(1, 3).zip(n(3, 5)).map_or(0, |(h, m)| sinal * (h * 60 + m))
    })
}

// Conversões de dia civil de Howard Hinnant (o mesmo cálculo do `chrono`).
fn dias_desde_1970(a: i64, m: i64, d: i64) -> i64 {
    let a = if m <= 2 { a - 1 } else { a };
    let era = a.div_euclid(400);
    let ano_era = a - era * 400;
    let dia_ano = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    era * 146_097 + ano_era * 365 + ano_era / 4 - ano_era / 100 + dia_ano - 719_468
}

fn data_civil(dias: i64) -> (i64, i64, i64) {
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let ano_era = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let dia_ano = doe - (365 * ano_era + ano_era / 4 - ano_era / 100);
    let mp = (5 * dia_ano + 2) / 153;
    let d = dia_ano - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (ano_era + era * 400 + i64::from(m <= 2), m, d)
}

/// `a` é mais nova que `b`? Compara número a número ("0.10.0" > "0.9.1").
pub fn mais_nova(a: &str, b: &str) -> bool {
    let partes = |s: &str| s.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    partes(a) > partes(b)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn data_no_fuso_de_brasilia() {
        assert_eq!(data_no_fuso("2026-10-20T00:23:34Z", -180), "19/10/2026");
        assert_eq!(data_no_fuso("2026-10-19T15:13:46Z", -180), "19/10/2026");
        assert_eq!(data_no_fuso("2026-01-01T01:00:00Z", -180), "31/12/2025");
        assert_eq!(data_no_fuso("2024-02-29T12:00:00Z", 0), "29/02/2024");
        assert_eq!(data_no_fuso("", 0), "—");
    }

    #[test]
    fn compara_versoes_por_numero() {
        assert!(mais_nova("0.10.0", "0.9.1"));
        assert!(mais_nova("0.1.4", "0.1.3"));
        assert!(!mais_nova("0.1.3", "0.1.3"));
    }

    #[test]
    fn acha_a_tag_no_feed() {
        let feed = r#"<feed><link href="https://github.com/x/y/releases"/><entry><link rel="alternate" type="text/html" href="https://github.com/x/y/releases/tag/v0.2.0"/></entry><entry><link href="https://github.com/x/y/releases/tag/v0.1.0"/></entry></feed>"#;
        assert_eq!(entrada_do_feed(feed).map(|e| e.0).as_deref(), Some("v0.2.0"));
        assert_eq!(entrada_do_feed("<feed></feed>"), None);
        let com_data = r#"<entry><updated>2026-10-20T00:23:34Z</updated><link href="https://github.com/x/y/releases/tag/v0.1.0"/></entry>"#;
        assert_eq!(entrada_do_feed(com_data), Some(("v0.1.0".into(), "2026-10-20T00:23:34Z".into())));
    }

    #[test]
    fn monta_a_release_pelo_sha256sums() {
        let somas = "aa  feathersheet-0.1.3-linux-x86_64.tar.gz\nbb  FeatherSheet-0.1.3-windows-x64.exe\ncc  dist-mac/FeatherSheet-0.1.3.dmg\n";
        let r = release_das_somas("v0.1.3", "https://x/", somas);
        assert_eq!(r.versao, "0.1.3");
        assert!(r.pacote.unwrap().url.starts_with("https://x/"));
        assert!(r.arquivos.iter().any(|a| a.nome == "SHA256SUMS") && r.arquivos.iter().any(|a| a.nome == "FeatherSheet-0.1.3.dmg"));
    }

    #[test]
    fn le_a_release_do_github() {
        let json = serde_json::json!({
            "tag_name": "v0.1.3", "published_at": "2026-10-19T15:13:46Z",
            "assets": [
                {"name": "SHA256SUMS", "browser_download_url": "u1", "size": 10},
                {"name": "feathersheet-linux-x86_64.tar.gz", "browser_download_url": "u2", "size": 99},
                {"name": "FeatherSheet-windows-x64.exe", "browser_download_url": "u3", "size": 98},
                {"name": "FeatherSheet-0.1.3-macos-apple-silicon.dmg", "browser_download_url": "u4", "size": 97}
            ]
        });
        let r = ler_release(&json);
        assert_eq!((r.versao.as_str(), r.data.as_str()), ("0.1.3", "19/10/2026"));
        assert!(r.pacote.unwrap().nome.ends_with(sufixo_do_sistema()));
    }
}
