const paths = {
  overview: 'M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z',
  server: 'M4 3h16v7H4z M4 14h16v7H4z M7 6.5h.01 M7 17.5h.01 M11 6.5h6 M11 17.5h6',
  services:
    'M12 3v4 M12 17v4 M3 12h4 M17 12h4 M5.6 5.6l2.8 2.8 M15.6 15.6l2.8 2.8 M5.6 18.4l2.8-2.8 M15.6 8.4l2.8-2.8 M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0',
  containers: 'M12 3l9 5v9l-9 5-9-5V8z M3 8l9 5 9-5 M12 13v9 M7.5 5.5l9 5',
  network: 'M9 3h6v6H9z M2 16h6v6H2z M16 16h6v6h-6z M12 9v4 M5 16v-3h14v3',
  security: 'M12 2l9 4v6c0 5-9 10-9 10S3 17 3 12V6z M8 12l3 3 5-6',
  logs: 'M6 3h12v18H6z M9 7h6 M9 11h6 M9 15h4',
  files: 'M3 6V4h7l2 3h9v13H3z',
  terminal: 'M3 4h18v16H3z M6 9l3 3-3 3 M12 15h5',
  plus: 'M12 5v14 M5 12h14',
  arrow: 'M5 12h14 M13 6l6 6-6 6',
  refresh: 'M20 7v5h-5 M4 17v-5h5 M19 11a7 7 0 0 0-12-5L4 9 M5 13a7 7 0 0 0 12 5l3-3',
  key: 'M14 10a5 5 0 1 1-4-5 M10 10l11 11 M16 16l3-3 M19 19l3-3',
  shield: 'M12 2l9 4v6c0 5-9 10-9 10S3 17 3 12V6z M12 8v5 M12 17h.01',
} as const;

export function Icon({ name, size = 18 }: { name: keyof typeof paths; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={paths[name]} />
    </svg>
  );
}
