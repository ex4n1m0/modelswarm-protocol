import type { Metadata } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: "ModelSwarm — Decentralized Cooperative Inference",
  description:
    "Network-aware micro-swarms of peers hosting the same immutable model profile. Lossless cooperative inference with fastest-host fallback. Download the Windows client.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
