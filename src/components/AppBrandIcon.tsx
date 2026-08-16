import { cn } from "../lib/utils";
import claudeSvg from "../assets/app-icons/claude.svg?raw";
import openaiSvg from "../assets/app-icons/openai.svg?raw";
import geminiSvg from "../assets/app-icons/gemini.svg?raw";
import grokSvg from "../assets/app-icons/grok.svg?raw";
import opencodeSvg from "../assets/app-icons/opencode.svg?raw";
import zcodeSvg from "../assets/app-icons/zcode.svg?raw";
import deepseekSvg from "../assets/app-icons/deepseek.svg?raw";
import hermesPng from "../assets/app-icons/hermes.png";

export type AppBrandIconName =
  | "claude"
  | "openai"
  | "gemini"
  | "grok"
  | "opencode"
  | "zcode"
  | "deepseek"
  | "hermes";

const ICONS: Record<Exclude<AppBrandIconName, "hermes">, string> = {
  claude: claudeSvg,
  openai: openaiSvg,
  gemini: geminiSvg,
  grok: grokSvg,
  opencode: opencodeSvg,
  zcode: zcodeSvg,
  deepseek: deepseekSvg,
};

interface Props {
  icon: AppBrandIconName;
  name: string;
  size?: number;
  className?: string;
}

export function AppBrandIcon({ icon, name, size = 16, className }: Props) {
  const containerClass = cn(
    "app-brand-icon inline-flex shrink-0 items-center justify-center",
    className,
  );
  const containerStyle = { width: size, height: size, fontSize: size };

  if (icon === "hermes") {
    return (
      <span aria-label={name} className={containerClass} role="img" style={containerStyle}>
        <img
          alt=""
          aria-hidden="true"
          className="block h-full max-h-full w-full max-w-full object-contain"
          src={hermesPng}
          style={{
            display: "block",
            width: "100%",
            height: "100%",
            maxWidth: "100%",
            maxHeight: "100%",
            objectFit: "contain",
          }}
        />
      </span>
    );
  }

  return (
    <span
      aria-label={name}
      className={containerClass}
      role="img"
      style={containerStyle}
      dangerouslySetInnerHTML={{ __html: ICONS[icon] }}
    />
  );
}
