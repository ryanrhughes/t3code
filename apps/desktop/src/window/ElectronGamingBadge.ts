import * as Electron from "electron";
import type { GamingBadge } from "./NativeGamingBadge.ts";

/** X11/native desktop fallback; Wayland uses a layer surface instead of this window. */
export async function startElectronGamingBadge(
  activate: () => void,
  platform: NodeJS.Platform,
): Promise<GamingBadge> {
  const badge = new Electron.BrowserWindow({
    width: 64,
    height: 64,
    frame: false,
    transparent: true,
    resizable: false,
    focusable: false,
    skipTaskbar: true,
    show: false,
    webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false },
  });
  const display = Electron.screen.getDisplayNearestPoint(Electron.screen.getCursorScreenPoint());
  const place = (corner: string) => {
    const { x, y, width, height } = display.bounds;
    badge.setPosition(
      corner.endsWith("left") ? x + 24 : x + width - 88,
      corner.startsWith("top") ? y + 96 : y + height - 88,
    );
  };
  try {
    if (platform === "linux" || platform === "win32") {
      const shape = Array.from({ length: 52 }, (_, row) => {
        const half = Math.floor(Math.sqrt(26 ** 2 - (row - 26) ** 2));
        return { x: 32 - half, y: row + 6, width: Math.max(1, half * 2), height: 1 };
      });
      badge.setShape(shape);
    }
    badge.setAlwaysOnTop(true, "screen-saver");
    badge.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
    badge.webContents.on("will-navigate", (event, url) => {
      event.preventDefault();
      if (url === "t3-gaming://activate") activate();
    });
    badge.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
    await badge.loadURL(
      `data:text/html;charset=utf-8,${encodeURIComponent(`<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'"><style>
      body{margin:4px;background:transparent;font-family:system-ui}a{box-sizing:border-box;display:flex;flex-direction:column;align-items:center;justify-content:center;width:56px;height:56px;background:#181d21;color:#ece4cf;border:3px solid var(--accent,#b59a63);border-radius:50%;font-weight:700;text-decoration:none;font-size:19px;line-height:22px}small{font-size:11px;line-height:13px;color:var(--accent,#b59a63)}.pulse{animation:pulse .64s 3}@keyframes pulse{50%{border-color:#fff2ce}}@media(prefers-reduced-motion:reduce){.pulse{animation:none}}
      </style><a href="t3-gaming://activate" title="Open T3 agents"><span>T3</span><small>•</small></a><script>
      window.updateBadge = s => { const a=document.querySelector('a');const n=s.attention+s.unread;a.style.setProperty('--accent',s.attention?'#edc46c':s.unread?'#94c7a3':s.working?'#8dc5e6':s.offline?'#aaa69c':'#b59a63');document.querySelector('small').textContent=n?Math.min(99,n):'•';a.title=s.attention+' need you · '+s.unread+' unread · '+s.working+' working · '+s.offline+' offline';if(s.pulse){a.classList.remove('pulse');void a.offsetWidth;a.classList.add('pulse')}};
      </script>`)}`,
    );
    place("top-right");
    badge.showInactive();
  } catch (error) {
    badge.destroy();
    throw error;
  }
  return {
    update(status) {
      if (badge.isDestroyed()) return;
      place(status.corner);
      void badge.webContents
        .executeJavaScript(`window.updateBadge(${JSON.stringify(status)})`)
        .catch(() => undefined);
    },
    close: async () => {
      if (!badge.isDestroyed()) badge.destroy();
    },
  };
}
