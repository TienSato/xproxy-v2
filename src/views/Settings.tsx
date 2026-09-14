import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ACCENTS, api, type Provider, type Settings, type TestResult } from "../lib/ipc";

export default function SettingsView({ settings, onSave }: {
  settings: Settings;
  onSave: (s: Settings) => Promise<void>;
}) {
  const [test, setTest] = useState<Record<string, TestResult | "…">>({});

  const patch = (p: Partial<Settings>) => onSave({ ...settings, ...p });
  const patchProvider = (id: string, p: Partial<Provider>) =>
    patch({ providers: settings.providers.map((x) => (x.id === id ? { ...x, ...p } : x)) });

  function addProvider() {
    const id = `p${Date.now().toString(36)}`;
    patch({
      providers: [...settings.providers, {
        id, name: "Nhà cung cấp mới", url: "", dashboard_url: "",
        upstream_scheme: "socks5", line_format: "host_first",
        vary_port: true, builtin: false,
      }],
      active_provider: id,
    });
  }

  function removeProvider(id: string) {
    const rest = settings.providers.filter((p) => p.id !== id);
    patch({
      providers: rest,
      active_provider: settings.active_provider === id ? rest[0]?.id ?? "" : settings.active_provider,
    });
  }

  async function runTest(id: string) {
    setTest((t) => ({ ...t, [id]: "…" }));
    try {
      const r = await api.testProvider(id);
      setTest((t) => ({ ...t, [id]: r }));
    } catch (e) {
      setTest((t) => ({ ...t, [id]: { url: "", body: String(e), parsed: [] } }));
    }
  }

  return (
    <div className="settings">
      <div className="settings-inner">

        {/* ── Giao diện ── */}
        <div className="section">
          <h3>Giao diện</h3>
          <p className="lede">Áp dụng ngay, lưu lại cho lần mở sau.</p>

          <div className="row">
            <span className="k">Chủ đề</span>
            <div className="v">
              <select value={settings.theme} onChange={(e) => patch({ theme: e.target.value as Settings["theme"] })}>
                <option value="system">Theo hệ thống</option>
                <option value="light">Sáng</option>
                <option value="dark">Tối</option>
              </select>
            </div>
          </div>

          <div className="row">
            <span className="k">Màu nhấn</span>
            <div className="v">
              <div className="swatches">
                {ACCENTS.map((c) => (
                  <button
                    key={c.value}
                    className="swatch"
                    style={{ background: c.value }}
                    aria-pressed={settings.accent === c.value}
                    title={c.name}
                    onClick={() => patch({ accent: c.value })}
                  />
                ))}
              </div>
            </div>
          </div>
        </div>

        {/* ── Nhà cung cấp ── */}
        <div className="section">
          <h3>Nhà cung cấp</h3>
          <p className="lede">
            Mỗi nhà cung cấp chỉ cần một URL lấy IP — dán nguyên từ trang của họ, key nằm
            luôn trong URL. Mỗi lần bấm Gán hoặc Đổi, app gọi URL này một lần và cắm dòng
            proxy trả về vào cổng.
          </p>

          {settings.providers.map((p) => {
            const t = test[p.id];
            return (
              <div key={p.id} className={`provider ${settings.active_provider === p.id ? "active" : ""}`}>
                <div className="provider-head">
                  <label className="inline">
                    <input
                      type="radio"
                      checked={settings.active_provider === p.id}
                      onChange={() => patch({ active_provider: p.id })}
                    />
                    Đang dùng
                  </label>
                  <input value={p.name} onChange={(e) => patchProvider(p.id, { name: e.target.value })} />
                  {!p.builtin && (
                    <button className="danger ghost" onClick={() => removeProvider(p.id)}>Xoá</button>
                  )}
                </div>

                <div className="row">
                  <span className="k">URL lấy IP</span>
                  <div className="v">
                    <input
                      className="grow sel"
                      placeholder="https://…/api/getIpInfo?key=…&port=443&num=1&country=US&type=2"
                      value={p.url}
                      onChange={(e) => patchProvider(p.id, { url: e.target.value })}
                    />
                  </div>
                </div>

                <div className="row">
                  <span className="k">Dòng trả về</span>
                  <div className="v">
                    <select
                      value={p.line_format}
                      onChange={(e) => patchProvider(p.id, { line_format: e.target.value as Provider["line_format"] })}
                    >
                      <option value="host_first">host:port:user:pass</option>
                      <option value="cred_at_host">user:pass@host:port</option>
                    </select>
                    <select
                      value={p.upstream_scheme}
                      onChange={(e) => patchProvider(p.id, { upstream_scheme: e.target.value as Provider["upstream_scheme"] })}
                    >
                      <option value="socks5">upstream SOCKS5</option>
                      <option value="http">upstream HTTP</option>
                    </select>
                  </div>
                </div>

                <div className="row">
                  <span className="k">Tham số port</span>
                  <div className="v">
                    <label className="inline">
                      <input
                        type="checkbox"
                        checked={p.vary_port}
                        onChange={(e) => patchProvider(p.id, { vary_port: e.target.checked })}
                      />
                      đổi theo từng cổng
                    </label>
                  </div>
                </div>

                <div className="row">
                  <span className="k">Trang quản lý</span>
                  <div className="v">
                    <input
                      className="grow"
                      placeholder="https://dash…"
                      value={p.dashboard_url}
                      onChange={(e) => patchProvider(p.id, { dashboard_url: e.target.value })}
                    />
                    <button onClick={() => p.dashboard_url && openUrl(p.dashboard_url)} disabled={!p.dashboard_url}>
                      Mở
                    </button>
                    <button onClick={() => runTest(p.id)} disabled={!p.url.trim()}>Thử URL</button>
                  </div>
                </div>

                {t === "…" && <div className="note-under">đang gọi…</div>}
                {t && t !== "…" && (
                  <>
                    <div
                      className="note-under"
                      style={{ color: t.parsed.length ? "var(--green)" : "var(--red)" }}
                    >
                      {t.parsed.length
                        ? `đọc được ${t.parsed.length} proxy — ${t.parsed.join(" · ")}`
                        : "không đọc được dòng proxy nào — kiểm tra URL hoặc đổi định dạng dòng trả về"}
                    </div>
                    <pre>{t.url}{"\n\n"}{t.body}</pre>
                  </>
                )}
              </div>
            );
          })}

          <button className="primary" onClick={addProvider}>+ Thêm nhà cung cấp</button>
        </div>

        {/* ── Cổng ── */}
        <div className="section">
          <h3>Cổng</h3>
          <p className="lede">
            App tự nhảy qua cổng đang bị chương trình khác giữ, nên số cổng bạn đặt luôn
            là số cổng dùng được.
          </p>

          <div className="row">
            <span className="k">Cổng bắt đầu</span>
            <div className="v">
              <input type="number" value={settings.start_port}
                onChange={(e) => patch({ start_port: Number(e.target.value) })} />
            </div>
          </div>
          <div className="row">
            <span className="k">Số lượng cổng</span>
            <div className="v">
              <input type="number" value={settings.port_count}
                onChange={(e) => patch({ port_count: Number(e.target.value) })} />
            </div>
          </div>
          <div className="row">
            <span className="k" />
            <div className="v">
              <button className="danger" onClick={() => api.generatePorts()}>Thiết lập lại dải cổng</button>
              <span className="hint">Dừng và tạo lại toàn bộ dải cổng.</span>
            </div>
          </div>
        </div>

        {/* ── Kết nối ── */}
        <div className="section">
          <h3>Kết nối</h3>

          <div className="row">
            <span className="k">Giao thức cổng LAN</span>
            <div className="v">
              <select value={settings.listen_scheme}
                onChange={(e) => patch({ listen_scheme: e.target.value as Settings["listen_scheme"] })}>
                <option value="socks5">SOCKS5</option>
                <option value="http">HTTP</option>
              </select>
              <span className="hint">Giao thức thiết bị nói với xproxy.</span>
            </div>
          </div>

          <div className="row">
            <span className="k">Timeout nối proxy</span>
            <div className="v">
              <input type="number" value={settings.dial_timeout_secs}
                onChange={(e) => patch({ dial_timeout_secs: Number(e.target.value) })} />
              <span className="hint">giây</span>
            </div>
          </div>

          <div className="row">
            <span className="k">Khi đổi IP</span>
            <div className="v">
              <select value={settings.drop_on_rotate ? "1" : "0"}
                onChange={(e) => patch({ drop_on_rotate: e.target.value === "1" })}>
                <option value="0">Kết nối đang mở chạy nốt trên IP cũ</option>
                <option value="1">Ngắt hết, buộc dùng IP mới ngay</option>
              </select>
            </div>
          </div>
        </div>

        {/* ── Chia tài nguyên ── */}
        <div className="section">
          <h3>Chia tài nguyên giữa các cổng</h3>

          <div className="row">
            <span className="k">Số kết nối mỗi cổng</span>
            <div className="v">
              <select
                value={settings.max_conns_per_slot === 0 ? "auto" : "manual"}
                onChange={(e) => patch({ max_conns_per_slot: e.target.value === "auto" ? 0 : 512 })}
              >
                <option value="auto">Tự cân bằng (khuyên dùng)</option>
                <option value="manual">Tự đặt</option>
              </select>
              {settings.max_conns_per_slot > 0 && (
                <>
                  <input type="number" value={settings.max_conns_per_slot}
                    onChange={(e) => patch({ max_conns_per_slot: Math.max(1, Number(e.target.value)) })} />
                  <span className="hint">kết nối / cổng</span>
                </>
              )}
            </div>
          </div>
          <div className="note-under">
            Máy còn rảnh thì cổng nào cần bao nhiêu cũng được. Khi cả app sắp chạm giới hạn
            của máy, mỗi cổng bị ghìm về phần chia đều của nó, nên cổng đang mở nhiều không
            lấn được sang cổng khác.
          </div>

          <div className="divider" />

          <div className="row">
            <span className="k">Trần băng thông</span>
            <div className="v">
              <input type="number" value={settings.rate_limit_kbps}
                onChange={(e) => patch({ rate_limit_kbps: Number(e.target.value) })} />
              <span className="hint">KB/s mỗi cổng — 0 là không giới hạn</span>
            </div>
          </div>
          <div className="note-under">
            Cứ để 0. Băng thông vốn đã chia tương đối đều giữa các kết nối. Chỉ đặt số khi
            muốn ghìm hẳn một cổng lại, ví dụ proxy tính tiền theo GB.
          </div>
        </div>

        {/* ── Tự động ── */}
        <div className="section">
          <h3>Tự động</h3>
          <p className="lede">
            App không tự gọi mạng qua proxy ở bất kỳ đâu. Muốn xem IP thoát thật thì bấm
            nút «IP?» trên đúng cổng đó — đấy là thao tác duy nhất tiêu băng thông.
          </p>

          <div className="row">
            <span className="k">Tự xoay IP</span>
            <div className="v">
              <label className="inline">
                <input type="checkbox" checked={settings.auto_rotate}
                  onChange={(e) => patch({ auto_rotate: e.target.checked })} />
                bật, mỗi
              </label>
              <input type="number" style={{ width: 80 }} value={settings.rotate_minutes}
                onChange={(e) => patch({ rotate_minutes: Number(e.target.value) })} />
              <span className="hint">phút — mỗi lần xoay tiêu 1 IP cho mỗi cổng đang chạy</span>
            </div>
          </div>

          <div className="row">
            <span className="k">Tự kiểm tra</span>
            <div className="v">
              <label className="inline">
                <input type="checkbox" checked={settings.auto_check}
                  onChange={(e) => patch({ auto_check: e.target.checked })} />
                bật, mỗi
              </label>
              <input type="number" style={{ width: 80 }} value={settings.check_interval_secs}
                onChange={(e) => patch({ check_interval_secs: Number(e.target.value) })} />
              <span className="hint">giây — chỉ bắt tay TCP, không tốn băng thông</span>
            </div>
          </div>
        </div>

      </div>
    </div>
  );
}
