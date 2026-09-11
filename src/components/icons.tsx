import mforkLogo from "../../design/mfork-logo.png";

interface IconProps {
  size?: number;
}

const base = (size: number) => ({
  width: size,
  height: size,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.7,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
});

export const ForkLogo = ({ size = 16 }: IconProps) => (
  <img src={mforkLogo} width={size} height={size} alt="Mfork" className="rounded-[22%]" />
);

export const IconOverview = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <rect x="3" y="3" width="7" height="7" rx="1.5" />
    <rect x="14" y="3" width="7" height="7" rx="1.5" />
    <rect x="3" y="14" width="7" height="7" rx="1.5" />
    <rect x="14" y="14" width="7" height="7" rx="1.5" />
  </svg>
);

export const IconProxies = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M3 12h18" />
    <ellipse cx="12" cy="12" rx="4" ry="9" />
  </svg>
);

export const IconProfiles = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
    <path d="M14 3v5h5" />
  </svg>
);

export const IconOverrides = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M4 6h9" />
    <circle cx="17" cy="6" r="2.5" />
    <path d="M20 12h-9" />
    <circle cx="7" cy="12" r="2.5" />
    <path d="M4 18h9" />
    <circle cx="17" cy="18" r="2.5" />
  </svg>
);

export const IconConnections = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M22 12h-4l-3 8-6-16-3 8H2" />
  </svg>
);

export const IconRules = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M21 5H3l7 8v5l4 2v-7z" />
  </svg>
);

export const IconLogs = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <path d="m5 7 5 5-5 5" />
    <path d="M13 17h7" />
  </svg>
);

export const IconSettings = ({ size = 17 }: IconProps) => (
  <svg {...base(size)}>
    <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
    <circle cx="12" cy="12" r="3" />
  </svg>
);
