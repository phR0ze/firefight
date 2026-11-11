import 'package:tray_manager/tray_manager.dart' as tray;
import 'package:window_manager/window_manager.dart' as wm;

/// Keeps the client display running in the system tray. The display runs for as long as the
/// firefight-agent that launches it, so it cannot be quit: closing the window only hides it, and
/// the tray icon has no menu. Any click on the icon brings the window back; without a menu, Linux
/// panels pass clicks on to the app (GNOME's AppIndicator extension only double clicks).
class ClientTray {
  ClientTray._(this._window, this._icon);

  final wm.Window _window;
  // Held only to stay alive: a collected wrapper releases its native handle and the icon vanishes.
  // ignore: unused_field
  final tray.TrayIcon _icon;

  /// Returns null when the platform offers no tray; see [keepOpen] for that case. The Linux
  /// runner handles `--start-hidden` itself, before the first frame.
  static ClientTray? start() {
    final window = wm.WindowManager.instance.getCurrent();
    final icon = tray.TrayIcon.create();
    if (window == null || icon == null) return null;

    final t = ClientTray._(window, icon);
    icon.icon = tray.ImageAsset.fromAsset('assets/tray_icon.png');
    icon.setTooltip('Firefight');
    icon.addListener((e) {
      if (e is tray.TrayIconClickedEvent ||
          e is tray.TrayIconRightClickedEvent ||
          e is tray.TrayIconDoubleClickedEvent) {
        t.show();
      }
    });
    icon.setVisible(true);

    // Closing the window hides it instead; the app keeps its agent connection and countdowns.
    window.addListener((event) {
      if (event is wm.WindowCloseRequestedEvent) {
        event.request.cancel();
        window.hide();
      }
    });
    return t;
  }

  /// Bring the window back to the front, e.g. from the tray or when a timer starts firing.
  void show() {
    _window.show();
    _window.focus();
  }

  /// Without a tray: closing the window minimizes it instead.
  static void keepOpen() {
    final window = wm.WindowManager.instance.getCurrent();
    window?.addListener((event) {
      if (event is wm.WindowCloseRequestedEvent) {
        event.request.cancel();
        window.minimize();
      }
    });
  }
}
