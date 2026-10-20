//! O catálogo da suíte e a consulta às releases do GitHub.
use feather_ui::marca::Produto;

pub const ORG: &str = "feather-works";
/// Começo da mensagem de cota estourada; a tela a reconhece por ele.
pub const LIMITE: &str = "o GitHub limitou as consultas";

pub struct App {
    pub id: &'static str,
    /// Segunda metade do nome ("Sheet" em "FeatherSheet"), pintada na cor do app.
    pub sufixo: &'static str,
    pub descricao: &'static str,
    /// `None` = sem logo própria no `feather-ui` ainda; o launcher recolore a pena.
    pub produto: Option<Produto>,
    pub cor: &'static str,
}

impl App {
    pub fn nome(&self) -> String {
        format!("Feather{}", self.sufixo)
    }
}

pub const APPS: [App; 5] = [
    App { id: "feathersheet", sufixo: "Sheet", descricao: "Planilhas e dados", produto: Some(Produto::FeatherSheet), cor: "#287846" },
    App { id: "featherslide", sufixo: "Slide", descricao: "Apresentações", produto: Some(Produto::FeatherSlide), cor: "#a87f0c" },
    App { id: "featherdocs", sufixo: "Docs", descricao: "Documentos de texto", produto: Some(Produto::FeatherDocs), cor: "#236ac7" },
    App { id: "featherpdf", sufixo: "PDF", descricao: "Leitura e organização de PDF", produto: None, cor: "#c5352b" },
    App { id: "feathercad", sufixo: "CAD", descricao: "Desenho técnico 2D e 3D", produto: None, cor: "#7048b8" },
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

pub fn cliente() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("featherlauncher/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("cliente HTTP")
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
    let Some(tag) = tag_do_feed(&feed) else { return Ok(None) };
    let base = format!("https://github.com/{ORG}/{id}/releases/download/{tag}/");
    let resp = cliente().get(format!("{base}SHA256SUMS")).send().await.map_err(|e| e.to_string())?;
    let somas = if resp.status().is_success() { resp.text().await.map_err(|e| e.to_string())? } else { String::new() };
    Ok(Some(release_das_somas(&tag, &base, &somas)))
}

/// A primeira entrada do feed é a release mais nova.
fn tag_do_feed(feed: &str) -> Option<String> {
    let entrada = &feed[feed.find("<entry>")?..];
    let resto = &entrada[entrada.find("/releases/tag/")? + "/releases/tag/".len()..];
    Some(resto[..resto.find(['"', '<'])?].to_string())
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

/// "2026-10-19T15:13:46Z" → "19/10/2026".
fn data_br(iso: &str) -> String {
    let d: Vec<&str> = iso.get(..10).unwrap_or_default().split('-').collect();
    match d[..] {
        [a, m, dia] => format!("{dia}/{m}/{a}"),
        _ => "—".into(),
    }
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
    fn compara_versoes_por_numero() {
        assert!(mais_nova("0.10.0", "0.9.1"));
        assert!(mais_nova("0.1.4", "0.1.3"));
        assert!(!mais_nova("0.1.3", "0.1.3"));
    }

    #[test]
    fn acha_a_tag_no_feed() {
        let feed = r#"<feed><link href="https://github.com/x/y/releases"/><entry><link rel="alternate" type="text/html" href="https://github.com/x/y/releases/tag/v0.2.0"/></entry><entry><link href="https://github.com/x/y/releases/tag/v0.1.0"/></entry></feed>"#;
        assert_eq!(tag_do_feed(feed).as_deref(), Some("v0.2.0"));
        assert_eq!(tag_do_feed("<feed></feed>"), None);
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
