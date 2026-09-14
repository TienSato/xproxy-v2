import { useEffect, useState } from "react";
import { api, onSlots, type Settings, type Slot, type Summary } from "./lib/ipc";
import { IconGear, IconList, Logo } from "./ui/icons";
import Forwarding from "./views/Forwarding";
import SettingsView from "./views/Settings";

type Tab = "forwarding" | "settings";

const EMPTY: Summary = {
  running: 0,
  total: 0,
  lan_ip: "—",
  health: { open_conns: 0, max_conns: 0, fd_limit: 0 },
  skipped_ports: [],
};

export default function App() {
  const [tab, setTab] = useState<Tab>("forwarding");
  const [slots, setSlots] = useState<Slot[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [sum, setSum] = useState<Summary>(EMPTY);

  // Rust đẩy danh sách cổng lên mỗi giây; UI chỉ ngồi nghe.
  useEffect(() => {
    const un = onSlots(setSlots);
    api.listSlots().then(setSlots).catch(() => {});
    api.getSettings().then(setSettings).catch(() => {});
    const poll = () => api.summary().then(setSum).catch(() => {});
    poll();
    const t = setInterval(poll, 1500);
    return () => {
      un.then((f) => f());
      clearInterval(t);
    };
  }, []);

  // Chủ đề + màu nhấn áp thẳng lên :root để mọi token CSS dùng chung.
  useEffect(() => {
    if (!settings) return;
    const root = document.documentElement;
    if (settings.theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", settings.theme);
    root.style.setProperty("--accent", settings.accent);
  }, [settings?.theme, settings?.accent]);

  async function saveSettings(next: Settings) {
    setSettings(next); // phản hồi ngay, không đợi vòng IPC
    await api.saveSettings(next);
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <Logo />
          <span>xproxy</span>
        </div>

        <button
          className={`navitem ${tab === "forwarding" ? "active" : ""}`}
          onClick={() => setTab("forwarding")}
        >
          <IconList />
          Danh sách chuyển tiếp
        </button>
        <button
          className={`navitem ${tab === "settings" ? "active" : ""}`}
          onClick={() => setTab("settings")}
        >
          <IconGear />
          Cài đặt
        </button>

        <div className="spacer" />

        <div className="statcard">
          <div className="sub">Cổng đang chạy</div>
          <div className="big">
            {sum.running} <span style={{ color: "var(--faint)", fontWeight: 400 }}>/ {sum.total}</span>
          </div>
          <div className="rule" />
          <div className="sub">IP LAN</div>
          <div className="sub mono" style={{ color: "var(--text)" }}>{sum.lan_ip}</div>
          <div className="rule" />
          <div className="sub">
            {sum.health.open_conns} kết nối · trần {sum.health.max_conns}
          </div>
        </div>
      </aside>

      <main className="main">
        {!settings ? (
          <div className="empty">Đang tải cấu hình…</div>
        ) : tab === "forwarding" ? (
          <Forwarding slots={slots} settings={settings} summary={sum} onSaveSettings={saveSettings} />
        ) : (
          <SettingsView settings={settings} onSave={saveSettings} />
        )}
      </main>
    </div>
  );
}
