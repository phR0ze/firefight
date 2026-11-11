import 'package:tray_manager/tray_manager.dart' as tray;
import 'package:window_manager/window_manager.dart' as wm;

/// Keeps the client display running in the system tray: closing the window only hides it, and
/// the tray menu brings it back or quits. Linux panels open the tray menu themselves and report
/// no icon clicks, so Show and Quit are menu items rather than click handlers.
///
/// A display supervised by firefight-agent (`--supervised`) runs for as long as the agent does,
/// so it has no Quit item.
class ClientTray {
  ClientTray._(this._window, this._icon, this._menu, this._items);

  final wm.Window _window;
  final tray.TrayIcon _icon;
  // Held only to stay alive: a collected wrapper releases its native handle and the menu entry
  // vanishes.
  // ignore: unused_field
  final tray.Menu _menu;
  // ignore: unused_field
  final List<tray.MenuItem> _items;
  bool _quitting = false;

  /// Returns null when the platform offers no tray, leaving the window's normal close behavior.
  static ClientTray? start({required bool startHidden, required bool supervised}) {
    final window = wm.WindowManager.instance.getCurrent();
    final icon = tray.TrayIcon.create();
    final menu = tray.Menu.create();
    final show = tray.MenuItem.createWithLabelAndType('Show', tray.MenuItemType.normal);
    final quit = supervised ? null : tray.MenuItem.createWithLabelAndType('Quit', tray.MenuItemType.normal);
    if (window == null || icon == null || menu == null || show == null || (!supervised && quit == null)) return null;

    final t = ClientTray._(window, icon, menu, [show, ?quit]);
    icon.icon = tray.ImageAsset.fromAsset('assets/tray_icon.png');
    icon.setTooltip('Firefight');
    show.addListener((e) {
      if (e is tray.MenuItemClickedEvent) t.show();
    });
    menu.addItem(show);
    if (quit != null) {
      quit.addListener((e) {
        if (e is tray.MenuItemClickedEvent) t.quit();
      });
      menu
        ..addSeparator()
        ..addItem(quit);
    }
    icon.setContextMenu(menu);
    icon.setContextMenuTrigger(tray.ContextMenuTrigger.clicked);
    icon.setVisible(true);

    // Closing the window hides it instead; the app keeps its agent connection and countdowns.
    window.addListener((event) {
      if (event is wm.WindowCloseRequestedEvent && !t._quitting) {
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

  /// For a supervised display without a tray: closing the window minimizes it instead.
  static void keepOpen() {
    final window = wm.WindowManager.instance.getCurrent();
    window?.addListener((event) {
      if (event is wm.WindowCloseRequestedEvent) {
        event.request.cancel();
        window.minimize();
      }
    });
  }

  void quit() {
    _quitting = true;
    _icon.dispose();
    _window.close();
  }
}
