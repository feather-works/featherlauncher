//! O catálogo da suíte e a consulta às releases do GitHub.
use feather_ui::marca::Produto;

pub const ORG: &str = "feather-works";

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
pub async fn ultima(id: &'static str) -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{ORG}/{id}/releases/latest");
    let resp = cliente().get(url).send().await.map_err(|e| format!("sem conexão: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub respondeu {}", resp.status()));
    }
    let corpo = resp.text().await.map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&corpo).map_err(|e| e.to_string())?;
    Ok(Some(ler_release(&json)))
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
