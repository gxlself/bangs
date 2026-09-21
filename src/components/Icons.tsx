import type { ComponentType, SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function Stroke({ children, ...props }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={16}
      height={16}
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      {...props}
    >
      {children}
    </svg>
  );
}

function Solid({ children, ...props }: IconProps) {
  return (
    <svg viewBox="0 0 24 24" width={16} height={16} fill="currentColor" aria-hidden {...props}>
      {children}
    </svg>
  );
}

// The tab glyphs are drawn for 13px: few strokes, nothing smaller than three units.
export const MusicIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="M4.5 13.5v-3M9.5 17V7M14.5 19.5v-15M19.5 14.5v-5" />
  </Stroke>
);

export const ShelfIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="M12 3.5v9m0 0 3.3-3.3M12 12.5 8.7 9.2" />
    <path d="M4 15v3.2A1.8 1.8 0 0 0 5.8 20h12.4a1.8 1.8 0 0 0 1.8-1.8V15" />
  </Stroke>
);

export const PinIcon = ({ filled, ...props }: IconProps & { filled?: boolean }) => (
  <Stroke {...props} fill={filled ? "currentColor" : "none"}>
    <path d="M12 16.4V21" />
    <path d="M8.4 3.2h7.2" />
    <path d="M9.9 3.2v6.4l-2.1 3.1a1 1 0 0 0 .83 1.56h6.74a1 1 0 0 0 .83-1.56L14.1 9.6V3.2" />
  </Stroke>
);

export const PlayIcon = (props: IconProps) => (
  <Solid {...props}>
    <path d="M7 4.8v14.4a1 1 0 0 0 1.52.85l11.5-7.2a1 1 0 0 0 0-1.7L8.52 3.95A1 1 0 0 0 7 4.8z" />
  </Solid>
);

export const PauseIcon = (props: IconProps) => (
  <Solid {...props}>
    <rect x="5.5" y="4" width="4.5" height="16" rx="1.2" />
    <rect x="14" y="4" width="4.5" height="16" rx="1.2" />
  </Solid>
);

export const NextIcon = (props: IconProps) => (
  <Solid {...props}>
    <path d="M3 5.7v12.6a1 1 0 0 0 1.55.83L13 13.5V18.3a1 1 0 0 0 1.55.83l8.44-5.6a1.8 1.8 0 0 0 0-3.06l-8.44-5.6A1 1 0 0 0 13 5.7v4.8L4.55 4.87A1 1 0 0 0 3 5.7z" />
  </Solid>
);

export const PreviousIcon = (props: IconProps) => (
  <NextIcon {...props} style={{ transform: "scaleX(-1)", ...props.style }} />
);

export const CloseIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="M18 6 6 18M6 6l12 12" />
  </Stroke>
);

export const FolderIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2z" />
  </Stroke>
);

// A shell prompt: the dev panel is a list of CLI sessions.
export const CodeIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="m5 8.5 4 3.5-4 3.5" />
    <path d="M12.5 16H19" />
  </Stroke>
);

// Two sheets stacked, not a clipboard with a clip: at 13px the clip merges
// into the outline and leaves an empty box.
export const ClipboardIcon = (props: IconProps) => (
  <Stroke {...props}>
    <rect x="8.6" y="3.2" width="11.2" height="13.4" rx="3" />
    <path d="M15.4 20.8H7a2.8 2.8 0 0 1-2.8-2.8V8.2" />
  </Stroke>
);

export const CheckIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="M20 6 9 17l-5-5" />
  </Stroke>
);

export const PlusIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="M12 5.5v13M5.5 12h13" />
  </Stroke>
);

/** A list with things ticked off it. */
export const TodoIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="m3.5 7.7 2.2 2.2 3.8-4.4" />
    <path d="m3.5 16.5 2.2 2.2 3.8-4.4" />
    <path d="M13.5 8.4H21M13.5 17.2H21" />
  </Stroke>
);

/** The plugin board: a slot other programs dock a row into. */
export const BoardIcon = (props: IconProps) => (
  <Stroke {...props}>
    <rect x="3" y="4.5" width="18" height="15" rx="3.2" />
    <path d="M7.5 9.5h9M7.5 14h5" />
  </Stroke>
);

export const RefreshIcon = (props: IconProps) => (
  <Stroke {...props}>
    <path d="M20 7v5h-5" />
    <path d="M19 12a7 7 0 1 0-1.8 4.7" />
  </Stroke>
);

/** The glyphs a plugin may name; anything else falls back to a dot. */
const GLYPHS: Record<string, ComponentType<IconProps>> = {
  board: BoardIcon,
  music: MusicIcon,
  shelf: ShelfIcon,
  code: CodeIcon,
  clipboard: ClipboardIcon,
  folder: FolderIcon,
  check: CheckIcon,
  todo: TodoIcon,
  pin: PinIcon,
};

export function GlyphFor({ name, ...props }: Omit<IconProps, "name"> & { name?: string | null }) {
  const Glyph = name ? GLYPHS[name] : undefined;
  if (Glyph) return <Glyph {...props} />;
  return (
    <Solid {...props} viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="4.5" />
    </Solid>
  );
}
