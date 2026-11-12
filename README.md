<p align="center">
  <img src="pacote/icones/featherlauncher.svg" width="88" alt="">
</p>

<h1 align="center">FeatherWorks Launcher</h1>

<p align="center">
  Instala, atualiza e abre os aplicativos da suíte FeatherWorks.
</p>

<p align="center">
  <img src="docs/launcher.png" width="820" alt="O launcher com o FeatherSheet e o FeatherSlide instalados">
</p>

## Baixar

| Sistema | Arquivo |
|---|---|
| Linux (Debian, Ubuntu, Mint…) | [featherlauncher-linux-amd64.deb](https://github.com/feather-works/featherlauncher/releases/latest/download/featherlauncher-linux-amd64.deb) |
| Linux (qualquer distribuição) | [featherlauncher-linux-x86_64.tar.gz](https://github.com/feather-works/featherlauncher/releases/latest/download/featherlauncher-linux-x86_64.tar.gz): descompacte e rode `./instalar.sh` |
| Windows 10/11 | [featherlauncher-windows-x64-instalador.exe](https://github.com/feather-works/featherlauncher/releases/latest/download/featherlauncher-windows-x64-instalador.exe): instala em `C:\Program Files\FeatherWorks\Launcher` (pede administrador uma vez). Portátil: [featherlauncher-windows-x64.exe](https://github.com/feather-works/featherlauncher/releases/latest/download/featherlauncher-windows-x64.exe), é só abrir |

## O que ele faz

- Lista os aplicativos da suíte com a versão instalada e a última publicada no GitHub.
- Instala e atualiza os apps: no Windows em `C:\Program Files\FeatherWorks\<App>` (o Windows pede administrador a cada instalação); no Linux na pasta do usuário (`~/.local`), sem administrador.
- No Linux, usa a cópia que já está instalada (`.deb`, Snap ou Flatpak) em vez de instalar outra: atualiza e abre essa cópia, mostra de onde ela veio e avisa quando o app está instalado mais de uma vez.
- Confere cada pacote pelo `SHA256SUMS` da release antes de instalar.
- Abre também em placa de vídeo antiga sem Vulkan/OpenGL utilizável (Intel HD 4000): a janela passa a ser desenhada pela CPU.
- Pausa e continua downloads; "Atualizar tudo" de uma vez.
- Consulta o GitHub com cache (ETag) e, se a cota da API acabar, pelo feed de releases.
- Sem conta, sem login, sem telemetria.

## Compilar

```bash
cargo run --release
pacote/gerar.sh          # pacotes em pacote/saida/
```

Licença: GPL-3.0.
