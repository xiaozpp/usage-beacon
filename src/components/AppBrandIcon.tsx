import { cn } from "../lib/utils";
import claudeSvg from "../assets/app-icons/claude.svg?raw";
import openaiSvg from "../assets/app-icons/openai.svg?raw";
import geminiSvg from "../assets/app-icons/gemini.svg?raw";
import grokSvg from "../assets/app-icons/grok.svg?raw";
import opencodeSvg from "../assets/app-icons/opencode.svg?raw";

export type AppBrandIconName = "claude" | "openai" | "gemini" | "grok" | "opencode";

const ICONS: Record<AppBrandIconName, string> = {
  claude: claudeSvg,
  openai: openaiSvg,
  gemini: geminiSvg,
  grok: grokSvg,
  opencode: opencodeSvg,
};

interface Props {
  icon: AppBrandIconName;
  name: string;
  size?: number;
  className?: string;
}

export function AppBrandIcon({ icon, name, size = 16, className }: Props) {
  return (
    <span
      aria-label={name}
      className={cn("app-brand-icon inline-flex shrink-0 items-center justify-center", className)}
      role="img"
      style={{ width: size, height: size, fontSize: size }}
      dangerouslySetInnerHTML={{ __html: ICONS[icon] }}
    />
  );
}
