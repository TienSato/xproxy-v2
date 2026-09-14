import { useEffect, useState } from "react";
import {
  api, COUNTRIES, humanBytes, onBatchProgress, statesFor,
  type Settings, type Slot, type Summary,
} from "../lib/ipc";

type Filter = "all" | "used" | "unused";

interface Props {
  slots: Slot[];
  settings: Settings;
  summary: Summary;
  onSaveSettings: (s: Settings) => Promise<void>;
}

export default function Forwarding({ slots, settings, summary, onSaveSettings }: Props) {
  const [filter, setFilter] = useState<Filter>("all");
  const [busy, setBusy] = useState<Set<number>>(new Set());
  const [batchMsg, setBatchMsg] = useState("");
  const [batchRunning, setBatchRunning] = useState(false);
  const [pasteFor, setPasteFor] = useState<number | null>(null);
  const [confirmBatch, setConfirmBatch] = useState<number | null>(null);

  const provider = settings.providers.find((p) => p.id === settings.active_provider);
  const ready = !!provider?.url.trim();

  useEffect(() => {
    const un = onBatchProgress(setBatchMsg);
    return () => { un.then((f) => f()); };
  }, []);

  const shown = slots.filter((s) =>
    filter === "all" ? true
      : filter === "used" ? s.status === "running" || s.status === "starting"
      : s.status !== "running" && s.status !== "starting"
  );
  const idleCount = slots.filter((s) => s.status !== "running" && s.status !== "starting").length;

  // Cảnh báo TRƯỚC khi các cổng bắt đầu chết vì cạn file descriptor.
  const pressure = summary.health.max_conns > 0
    ? summary.health.open_conns / summary.health.max_conns
    : 0;

  function mark(port: number, on: boolean) {
    setBusy((b) => {
      const n = new Set(b);
      if (on) n.add(port); else n.delete(port);
      return n;
    });
  }

  async function assign(port: number) {
    mark(port, true);
    try { await api.assignSlot(port); } catch { /* lỗi đã nằm trong message của cổng */ }
    mark(port, false);
  }

  async function runBatch(n: number) {
    setBatchRunning(true);
    try { setBatchMsg(await api.assignBatch(n)); } finally { setBatchRunning(false); }
    setTimeout(() => setBatchMsg(""), 4000);
  }

  const patch = (p: Partial<Settings>) => onSaveSettings({ ...settings, ...p });

  return (
    <>
      <div className="bar">
        <div className="field">
          <label>Nhà cung cấp</label>
          <select value={settings.active_provider} onChange={(e) => patch({ active_provider: e.target.value })}>
            {settings.providers.map((p) => (
              <option key={p.id} value={p.id}>{p.name}{p.url.trim() ? "" : " (chưa có URL)"}</option>
            ))}
          </select>
        </div>
        <div className="field">
          <label>Quốc gia</label>
          <select value={settings.country} onChange={(e) => patch({ country: e.target.value, state: "", city: "" })}>
            <option value="">(theo URL)</option>
            {COUNTRIES.map((c) => <option key={c.code} value={c.code}>{c.name} ({c.code})</option>)}
          </select>
        </div>
        <div className="field">
          <label>Bang / tỉnh</label>
          {statesFor(settings.country).length ? (
            <select value={settings.state} onChange={(e) => patch({ state: e.target.value })}>
              <option value="">(theo URL)</option>
              {statesFor(settings.country).map((s) => <option key={s} value={s}>{s}</option>)}
            </select>
          ) : (
            <input style={{ width: 130 }} value={settings.state} placeholder="(theo URL)"
              onChange={(e) => patch({ state: e.target.value })} />
          )}
        </div>
        <div className="field">
          <label>Thành phố</label>
          <input style={{ width: 120 }} value={settings.city} placeholder="(theo URL)"
            onChange={(e) => patch({ city: e.target.value })} />
        </div>
        <div className="grow" />
        <span className="hint">Để trống = giữ nguyên tham số có sẵn trong URL.</span>
      </div>

      {summary.skipped_ports.length > 0 && (
        <div className="banner">
          Đã tự nhảy qua cổng {summary.skipped_ports.join(", ")} — chương trình khác đang
          giữ. Trên macOS cổng 5000 và 7000 thường là AirPlay Receiver.
        </div>
      )}
      {!ready && (
        <div className="banner warn">
          Nhà cung cấp «{provider?.name}» chưa có URL lấy IP. Vào Cài đặt → dán URL rồi bấm «Thử URL».
        </div>
      )}
      {pressure > 0.8 && (
        <div className="banner warn">
          Sắp chạm trần kết nối của cả app ({summary.health.open_conns}/{summary.health.max_conns}).
          Hạ «Trần kết nối mỗi cổng» trong Cài đặt, nếu không các cổng sẽ bắt đầu từ chối kết nối.
        </div>
      )}

      <div className="bar">
        <select
          value=""
          disabled={batchRunning || idleCount === 0 || !ready}
          onChange={(e) => { if (e.target.value) setConfirmBatch(Number(e.target.value)); }}
        >
          <option value="">Gán hàng loạt…</option>
          {[10, 20, 25, 50].map((n) => <option key={n} value={n}>Gán {n} cổng trống</option>)}
          <option value={idleCount}>Gán tất cả ({idleCount})</option>
        </select>
        <span className="hint">{idleCount} cổng trống</span>

        <select value={filter} onChange={(e) => setFilter(e.target.value as Filter)}>
          <option value="all">Tất cả cổng</option>
          <option value="used">Đang dùng</option>
          <option value="unused">Chưa dùng</option>
        </select>

        <button onClick={() => api.checkAll()}>Kiểm tra ngay</button>
        <button className="danger" onClick={() => api.stopAll()}>Dừng tất cả</button>

        <div className="grow" />
        <span className="hint mono">
          {summary.health.open_conns} kết nối · trần {summary.health.max_conns}
        </span>
        {batchRunning && <button className="danger" onClick={() => api.cancelBatch()}>Dừng đợt gán</button>}
        {batchMsg && <span className="hint">{batchMsg}</span>}
      </div>

      <div className="tablewrap">
        <table>
          <thead>
            <tr>
              <th style={{ width: 70 }}>Cổng</th>
              <th style={{ width: 190 }}>Trạng thái</th>
              <th style={{ width: 110 }}>Thiết bị</th>
              <th style={{ width: 150 }}>↑ / ↓</th>
              <th>Ghi chú</th>
              <th style={{ width: 210 }}>Thao tác</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((s) => (
              <Row
                key={s.port}
                slot={s}
                busy={busy.has(s.port)}
                canAssign={ready}
                onAssign={() => assign(s.port)}
                onUnassign={() => api.unassignSlot(s.port)}
                onPaste={() => setPasteFor(s.port)}
              />
            ))}
          </tbody>
        </table>
        {shown.length === 0 && <div className="empty">Không có cổng nào. Vào Cài đặt → Thiết lập cổng.</div>}
      </div>

      {pasteFor !== null && <PasteDialog port={pasteFor} onClose={() => setPasteFor(null)} />}

      {confirmBatch !== null && (
        <div className="backdrop" onClick={() => setConfirmBatch(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>Xác nhận gán hàng loạt</h3>
            <p>
              Sẽ gán {Math.min(confirmBatch, idleCount)} cổng. Mỗi cổng là một lần gọi URL
              của {provider?.name} = tiêu một IP trong tài khoản.
            </p>
            <div className="actions">
              <button onClick={() => setConfirmBatch(null)}>Huỷ</button>
              <button className="primary" onClick={() => { const n = confirmBatch; setConfirmBatch(null); runBatch(n); }}>
                Xác nhận
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

function Row({ slot, busy, canAssign, onAssign, onUnassign, onPaste }: {
  slot: Slot; busy: boolean; canAssign: boolean;
  onAssign: () => void; onUnassign: () => void; onPaste: () => void;
}) {
  const [checking, setChecking] = useState(false);
  const running = slot.status === "running";
  const devices = slot.stats.devices;

  // Người dùng chỉ cần biết cổng này đang ở tình trạng nào, không cần nhìn host:port
  // của nhà cung cấp.
  const st = (() => {
    if (slot.status === "error") return { cls: "err", dot: "red", label: "Lỗi" };
    if (slot.status === "starting") return { cls: "wait", dot: "orange", label: "Đang lấy IP…" };
    if (running && devices > 0) return { cls: "on", dot: "green", label: "Đã kết nối" };
    if (running && slot.alive === false) return { cls: "err", dot: "red", label: "Proxy không phản hồi" };
    if (running) return { cls: "on", dot: "green", label: "Sẵn sàng" };
    if (slot.status === "listening") return { cls: "off", dot: "gray", label: "Cổng mở, chưa có IP" };
    return { cls: "off", dot: "gray", label: "Chưa dùng" };
  })();

  async function checkIp() {
    setChecking(true);
    try { await api.checkExitIp(slot.port); } catch { /* lỗi hiện ở cột ghi chú */ }
    setChecking(false);
  }

  const note = slot.status === "error"
    ? slot.message
    : [slot.exit_ip && `IP thoát ${slot.exit_ip}`, slot.region_text, slot.message]
        .filter(Boolean).join(" · ");

  return (
    <tr>
      <td className="mono sel">{slot.port}</td>

      <td>
        <span className={`state ${st.cls}`}>
          <span className={`dot ${st.dot}`} />
          <span className="label">{st.label}</span>
        </span>
      </td>

      <td
        className="mono"
        title={
          `${slot.stats.open} kết nối TCP đang mở · ${slot.stats.total} đã phục vụ` +
          (slot.stats.churn ? ` · ${slot.stats.churn} đóng sớm (bình thường)` : "") +
          (slot.stats.udp_refused ? ` · ${slot.stats.udp_refused} lần xin UDP (bình thường)` : "") +
          (slot.stats.rejected ? ` · ${slot.stats.rejected} bị chặn vì chạm trần` : "")
        }
      >
        {devices > 0 ? `${devices} thiết bị` : <span style={{ color: "var(--faint)" }}>—</span>}
      </td>

      <td
        className="mono"
        onDoubleClick={() => api.resetTraffic(slot.port)}
        title="nháy đúp để xoá số liệu"
      >
        {humanBytes(slot.stats.up)} / {humanBytes(slot.stats.down)}
      </td>

      <td>
        <div className="note" title={note}>
          {slot.status === "error"
            ? <span style={{ color: "var(--red)" }}>{note}</span>
            : (note || <span style={{ color: "var(--faint)" }}>—</span>)}
          {slot.stats.failed > 0 && (
            <span
              className="hint warn"
              title="Không nối được tới proxy: hết thời gian chờ, bị từ chối, hoặc sai user/pass. Nếu con số này tăng nhanh thì IP đang gán có vấn đề — bấm Đổi."
            >
              {" "}·{slot.stats.failed} lỗi
            </span>
          )}
        </div>
      </td>

      <td>
        <div className="rowbtns">
          <button onClick={onAssign} disabled={busy || !canAssign}>{running ? "Đổi" : "Gán"}</button>
          <button className="ghost" onClick={onPaste}>Dán</button>
          <button
            className="ghost"
            onClick={checkIp}
            disabled={!running || checking}
            title="Gọi thử qua proxy để xem IP thoát thật. Đây là thao tác DUY NHẤT tốn băng thông nhà cung cấp."
          >
            {checking ? "…" : "IP?"}
          </button>
          <button className="ghost danger" onClick={onUnassign} disabled={slot.status === "idle"}>Gỡ</button>
        </div>
      </td>
    </tr>
  );
}

function PasteDialog({ port, onClose }: { port: number; onClose: () => void }) {
  const [line, setLine] = useState("");
  const [atHost, setAtHost] = useState(false);
  const [err, setErr] = useState("");

  async function go() {
    try { await api.assignManual(port, line, atHost); onClose(); }
    catch (e) { setErr(String(e)); }
  }

  return (
    <div className="backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Dán proxy cho cổng {port}</h3>
        <select value={atHost ? "1" : "0"} onChange={(e) => setAtHost(e.target.value === "1")}>
          <option value="0">host:port:user:pass</option>
          <option value="1">user:pass@host:port</option>
        </select>
        <input
          autoFocus className="sel"
          placeholder="64.205.177.44:443:6ab9171c2721211f:kd7c2apyfaxslfb61syy"
          value={line}
          onChange={(e) => { setLine(e.target.value); setErr(""); }}
          onKeyDown={(e) => { if (e.key === "Enter" && line.trim()) go(); }}
        />
        {err && <div className="hint" style={{ color: "var(--red)" }}>{err}</div>}
        <div className="actions">
          <button onClick={onClose}>Huỷ</button>
          <button className="primary" onClick={go} disabled={!line.trim()}>Gán vào cổng {port}</button>
        </div>
      </div>
    </div>
  );
}
