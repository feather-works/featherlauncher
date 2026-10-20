//! O reqwest e o `spawn_blocking` precisam do tokio; o pelicano roda as tarefas num pool
//! próprio. Estas pontes rodam o trabalho no tokio e entregam o resultado ao pelicano.
//! Largar o futuro (ou o fluxo) cancela a tarefa do tokio, como o abort do Iced fazia.
use pelicano::futures::channel::mpsc::{channel, Sender};
use pelicano::futures::{Stream, StreamExt};
use std::future::Future;
use std::sync::OnceLock;

fn tokio() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime do tokio"))
}

struct Cancela(tokio::task::AbortHandle);

impl Drop for Cancela {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Roda `f` no tokio.
pub fn no_tokio<T: Send + 'static>(f: impl Future<Output = T> + Send + 'static) -> impl Future<Output = T> + Send + 'static {
    let tarefa = tokio().spawn(f);
    let cancela = Cancela(tarefa.abort_handle());
    async move {
        let r = tarefa.await;
        drop(cancela);
        r.expect("a tarefa do tokio terminou com pânico")
    }
}

/// Um fluxo produzido no tokio: `f` recebe a ponta que envia.
pub fn fluxo<T: Send + 'static, F: Future<Output = ()> + Send + 'static>(tamanho: usize, f: impl FnOnce(Sender<T>) -> F) -> impl Stream<Item = T> + Send + 'static {
    let (saida, entrada) = channel(tamanho);
    let cancela = Cancela(tokio().spawn(f(saida)).abort_handle());
    entrada.map(move |v| {
        let _ = &cancela;
        v
    })
}

/// Um tique a cada `periodo`, sem ocupar thread dormindo.
pub fn relogio(periodo: std::time::Duration) -> impl Stream<Item = ()> + Send + 'static {
    fluxo(1, move |mut saida| async move {
        use pelicano::futures::SinkExt;
        let mut t = tokio::time::interval(periodo);
        loop {
            t.tick().await;
            if saida.send(()).await.is_err() {
                return;
            }
        }
    })
}
