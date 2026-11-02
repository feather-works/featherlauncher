//! Canto arredondado no Windows 10. Lá a janela do pelicano não é transparente (o fundo fora do
//! canto saía preto), então o canto vem de recortar a própria janela (`SetWindowRgn`).
//! No Windows 11 o DWM já arredonda e recortar tiraria a sombra dele: lá não mexe.

use pelicano::raw_window_handle::RawWindowHandle;
use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
use windows_sys::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsZoomed};

/// Recorta a janela no raio `raio` (lógico); maximizada, volta ao retângulo.
pub fn recortar(alca: RawWindowHandle, raio: f32) {
    let RawWindowHandle::Win32(w) = alca else { return };
    if windows_11() {
        return;
    }
    let hwnd = w.hwnd.get() as _;
    // SAFETY: hwnd é a janela viva do pelicano, chamado na thread dela (Task::window_handle).
    // A região passa a ser do sistema depois do SetWindowRgn: não se apaga.
    unsafe {
        // Sem pedir redesenho (último 0): o pelicano já redesenha a cada mudança de tamanho, e
        // pedir de novo dobrava o trabalho de cada passo ao redimensionar.
        if IsZoomed(hwnd) != 0 {
            SetWindowRgn(hwnd, std::ptr::null_mut(), 0);
            return;
        }
        let mut r = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) == 0 {
            return;
        }
        let d = (2.0 * raio * GetDpiForWindow(hwnd).max(96) as f32 / 96.0).round() as i32;
        SetWindowRgn(hwnd, CreateRoundRectRgn(0, 0, r.right - r.left + 1, r.bottom - r.top + 1, d, d), 0);
    }
}

/// Windows 11 = build 22000 em diante (com o manifesto, o GetVersionExW dá o número real).
fn windows_11() -> bool {
    // SAFETY: estrutura zerada com o tamanho certo.
    let mut v: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    v.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
    (unsafe { GetVersionExW(&mut v) }) != 0 && v.dwBuildNumber >= 22000
}
