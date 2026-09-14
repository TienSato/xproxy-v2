import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ── Kiểu dữ liệu, khớp với struct bên Rust ──────────────────────────────────

export type Scheme = "socks5" | "http";
export type LineFormat = "host_first" | "cred_at_host";
export type SlotStatus = "idle" | "listening" | "starting" | "running" | "error";

export interface Stats {
  up: number;
  down: number;
  open: number;
  total: number;
  failed: number;
  /** Kết nối bị từ chối vì chạm trần (của cổng hoặc của cả app). */
  rejected: number;
  /** Lần client xin UDP/BIND — bình thường, không phải lỗi. */
  udp_refused: number;
  /** Kết nối rớt kiểu bình thường (client đóng, peer reset) — không phải lỗi. */
  churn: number;
  /** Số THIẾT BỊ đang cắm vào (đếm theo IP), không phải số kết nối TCP. */
  devices: number;
}

export interface Slot {
  port: number;
  provider_port: number;
  provider_id: string;
  country: string;
  state: string;
  city: string;
  status: SlotStatus;
  /** host:port proxy nhà cung cấp trả về — đến từ chính phản hồi API, không tốn gì để biết. */
  upstream_host: string;
  upstream_port: number;
  /** IP thoát thật. Chỉ có khi người dùng tự bấm kiểm tra (đi qua proxy, tốn băng thông). */
  exit_ip: string;
  alive: boolean | null;
  message: string;
  stats: Stats;
  region_text: string;
}

export interface Provider {
  id: string;
  name: string;
  /** URL lấy IP, dán nguyên từ trang nhà cung cấp. Key nằm luôn trong URL. */
  url: string;
  dashboard_url: string;
  upstream_scheme: Scheme;
  line_format: LineFormat;
  /** Nếu URL có tham số `port`, đổi nó theo từng cổng. */
  vary_port: boolean;
  builtin: boolean;
}

export interface Settings {
  active_provider: string;
  providers: Provider[];
  start_port: number;
  port_count: number;
  listen_scheme: Scheme;
  drop_on_rotate: boolean;
  dial_timeout_secs: number;
  max_conns_per_slot: number;
  /** Trần băng thông mỗi cổng, KB/s. 0 = không giới hạn. */
  rate_limit_kbps: number;
  auto_rotate: boolean;
  rotate_minutes: number;
  auto_check: boolean;
  check_interval_secs: number;
  country: string;
  state: string;
  city: string;
  theme: "system" | "light" | "dark";
  accent: string;
}

/** Màu nhấn chọn sẵn. Tên tiếng Việt vì hiện thẳng ra tooltip. */
export const ACCENTS: Array<{ name: string; value: string }> = [
  { name: "Xanh dương", value: "#2563eb" },
  { name: "Tím", value: "#7c3aed" },
  { name: "Xanh ngọc", value: "#0d9488" },
  { name: "Xanh lá", value: "#16a34a" },
  { name: "Cam", value: "#ea580c" },
  { name: "Hồng", value: "#db2777" },
  { name: "Xám đá", value: "#475569" },
];

export interface Health {
  open_conns: number;
  max_conns: number;
  fd_limit: number;
}

export interface Summary {
  running: number;
  total: number;
  lan_ip: string;
  health: Health;
  /** Cổng app đã tự nhảy qua vì chương trình khác đang giữ. */
  skipped_ports: number[];
}

export interface TestResult {
  url: string;
  body: string;
  /** Những dòng app đọc được. Trống = URL sai hoặc chọn nhầm định dạng dòng. */
  parsed: string[];
}

// ── Lệnh ────────────────────────────────────────────────────────────────────

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (value: Settings) => invoke<void>("save_settings", { value }),
  listProviders: () => invoke<Provider[]>("list_providers"),
  testProvider: (providerId: string) => invoke<TestResult>("test_provider", { providerId }),
  lanIp: () => invoke<string>("lan_ip"),

  generatePorts: () => invoke<Slot[]>("generate_ports"),
  listSlots: () => invoke<Slot[]>("list_slots"),
  summary: () => invoke<Summary>("summary"),

  assignSlot: (port: number) => invoke<Slot>("assign_slot", { port }),
  assignManual: (port: number, line: string, credAtHost: boolean) =>
    invoke<Slot>("assign_manual", { port, line, credAtHost }),
  unassignSlot: (port: number) => invoke<void>("unassign_slot", { port }),
  stopAll: () => invoke<void>("stop_all"),
  assignBatch: (count: number) => invoke<string>("assign_batch", { count }),
  cancelBatch: () => invoke<void>("cancel_batch"),

  checkSlot: (port: number) => invoke<boolean | null>("check_slot", { port }),
  checkAll: () => invoke<void>("check_all"),
  /** Lệnh DUY NHẤT gửi dữ liệu qua proxy. Chỉ gọi khi người dùng bấm. */
  checkExitIp: (port: number) => invoke<string>("check_exit_ip", { port }),
  resetTraffic: (port: number) => invoke<void>("reset_traffic", { port }),
};

/// Rust đẩy danh sách cổng lên mỗi giây — UI không phải tự hỏi vòng.
export const onSlots = (cb: (slots: Slot[]) => void) => listen<Slot[]>("slots", (e) => cb(e.payload));
export const onBatchProgress = (cb: (msg: string) => void) =>
  listen<string>("batch-progress", (e) => cb(e.payload));

// ── Tiện ích hiển thị ───────────────────────────────────────────────────────

export function humanBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

export const COUNTRIES: Array<{ code: string; name: string }> = [
  { code: "US", name: "United States" }, { code: "GB", name: "United Kingdom" },
  { code: "CA", name: "Canada" }, { code: "AU", name: "Australia" },
  { code: "DE", name: "Germany" }, { code: "FR", name: "France" },
  { code: "NL", name: "Netherlands" }, { code: "JP", name: "Japan" },
  { code: "KR", name: "South Korea" }, { code: "SG", name: "Singapore" },
  { code: "HK", name: "Hong Kong" }, { code: "TW", name: "Taiwan" },
  { code: "VN", name: "Vietnam" }, { code: "TH", name: "Thailand" },
  { code: "ID", name: "Indonesia" }, { code: "IN", name: "India" },
  { code: "BR", name: "Brazil" }, { code: "ES", name: "Spain" },
  { code: "IT", name: "Italy" }, { code: "RU", name: "Russia" },
];

export const US_STATES = [
  "Alabama","Alaska","Arizona","Arkansas","California","Colorado","Connecticut","Delaware",
  "Florida","Georgia","Hawaii","Idaho","Illinois","Indiana","Iowa","Kansas","Kentucky",
  "Louisiana","Maine","Maryland","Massachusetts","Michigan","Minnesota","Mississippi",
  "Missouri","Montana","Nebraska","Nevada","New Hampshire","New Jersey","New Mexico",
  "New York","North Carolina","North Dakota","Ohio","Oklahoma","Oregon","Pennsylvania",
  "Rhode Island","South Carolina","South Dakota","Tennessee","Texas","Utah","Vermont",
  "Virginia","Washington","West Virginia","Wisconsin","Wyoming",
];

export const CA_PROVINCES = [
  "Alberta","British Columbia","Manitoba","New Brunswick","Newfoundland and Labrador",
  "Northwest Territories","Nova Scotia","Nunavut","Ontario","Prince Edward Island",
  "Quebec","Saskatchewan","Yukon",
];

export const AU_STATES = [
  "Australian Capital Territory","New South Wales","Northern Territory","Queensland",
  "South Australia","Tasmania","Victoria","Western Australia",
];

export function statesFor(country: string): string[] {
  if (country === "US") return US_STATES;
  if (country === "CA") return CA_PROVINCES;
  if (country === "AU") return AU_STATES;
  return [];
}
