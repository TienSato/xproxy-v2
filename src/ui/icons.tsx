/** Bộ icon tự vẽ — không kéo thư viện icon về chỉ để dùng bốn hình. */

/** Logo xproxy: một cổng vào, nhiều đường ra — đúng thứ app này làm. */
export function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden>
      <rect x="1" y="1" width="22" height="22" rx="6.5" fill="var(--accent)" />
      <circle cx="7" cy="12" r="2.1" fill="var(--accent-text)" />
      <circle cx="17" cy="6.6" r="1.7" fill="var(--accent-text)" opacity=".92" />
      <circle cx="17" cy="12" r="1.7" fill="var(--accent-text)" opacity=".92" />
      <circle cx="17" cy="17.4" r="1.7" fill="var(--accent-text)" opacity=".92" />
      <path
        d="M9.1 12h2.4M11.5 12c2 0 1.6-5.4 3.8-5.4M11.5 12h3.8M11.5 12c2 0 1.6 5.4 3.8 5.4"
        stroke="var(--accent-text)"
        strokeWidth="1.3"
        strokeLinecap="round"
        opacity=".92"
      />
    </svg>
  );
}

export function IconList({ size = 15 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden>
      <rect x="1.5" y="2.5" width="13" height="3" rx="1.2" stroke="currentColor" strokeWidth="1.3" />
      <rect x="1.5" y="8.5" width="13" height="3" rx="1.2" stroke="currentColor" strokeWidth="1.3" />
      <circle cx="12" cy="4" r=".9" fill="currentColor" />
      <circle cx="12" cy="10" r=".9" fill="currentColor" />
    </svg>
  );
}

export function IconGear({ size = 15 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden>
      <circle cx="8" cy="8" r="2.3" stroke="currentColor" strokeWidth="1.3" />
      <path
        d="M8 1.6v1.7M8 12.7v1.7M14.4 8h-1.7M3.3 8H1.6M12.5 3.5l-1.2 1.2M4.7 11.3l-1.2 1.2M12.5 12.5l-1.2-1.2M4.7 4.7L3.5 3.5"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  );
}
