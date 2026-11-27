import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Copy } from "lucide-react";
import { isValidElement, type ReactNode, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Button } from "@/components/ui/button";

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}

function CodeBlock({ children }: { children?: ReactNode }) {
  const [copied, setCopied] = useState(false);
  const code = textOf(children).replace(/\n$/, "");

  async function copy() {
    await navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="group relative my-3">
      <pre className="overflow-x-auto rounded-md border border-border bg-muted p-3 pr-20 font-mono text-[13px] leading-relaxed">
        {children}
      </pre>
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={copy}
        className="absolute top-2 right-2 h-7 px-2 text-xs"
      >
        {copied ? <Check /> : <Copy />}
        {copied ? "Copied" : "Copy"}
      </Button>
    </div>
  );
}

function ExternalLink({ href, children }: { href?: string; children?: ReactNode }) {
  if (!href || !/^https?:\/\//i.test(href)) {
    return <span>{children}</span>;
  }
  return (
    <a
      href={href}
      onClick={(event) => {
        event.preventDefault();
        void openUrl(href);
      }}
      className="text-primary underline underline-offset-2"
    >
      {children}
    </a>
  );
}

function ImageAsLink({ src, alt }: { src?: string | Blob; alt?: string }) {
  const url = typeof src === "string" ? src : "";
  return (
    <span>
      {alt ? `${alt} ` : null}
      {url ? <ExternalLink href={url}>{url}</ExternalLink> : null}
    </span>
  );
}

const components: Components = {
  a: ExternalLink,
  img: ImageAsLink,
  pre: CodeBlock,
  code: ({ className, children }) => (
    <code
      className={className ? "font-mono" : "rounded bg-muted px-1 py-0.5 font-mono text-[0.9em]"}
    >
      {children}
    </code>
  ),
  p: ({ children }) => <p className="my-2 leading-relaxed">{children}</p>,
  ul: ({ children }) => <ul className="my-2 list-disc pl-6">{children}</ul>,
  ol: ({ children }) => <ol className="my-2 list-decimal pl-6">{children}</ol>,
  h1: ({ children }) => <h3 className="mt-4 mb-2 text-xl font-semibold">{children}</h3>,
  h2: ({ children }) => <h4 className="mt-4 mb-2 text-lg font-semibold">{children}</h4>,
  h3: ({ children }) => <h5 className="mt-3 mb-1 font-semibold">{children}</h5>,
  blockquote: ({ children }) => (
    <blockquote className="my-2 border-l-2 border-border pl-3 text-muted-foreground">
      {children}
    </blockquote>
  ),
  table: ({ children }) => (
    <div className="my-3 overflow-x-auto">
      <table className="border-collapse text-sm">{children}</table>
    </div>
  ),
  th: ({ children }) => (
    <th className="border border-border bg-muted px-2 py-1 text-left">{children}</th>
  ),
  td: ({ children }) => <td className="border border-border px-2 py-1">{children}</td>,
};

export function Markdown({ text }: { text: string }) {
  return (
    <div className="min-w-0 break-words">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
}
