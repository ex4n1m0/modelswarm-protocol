import type { Metadata } from "next";
import type { ReactNode } from "react";

const SITE_URL = "https://modelswarm.deepflux.space";
const TITLE = "ModelSwarm — Decentralized Cooperative Inference";
const DESCRIPTION =
  "Network-aware micro-swarms of peers hosting the same immutable model profile. " +
  "Lossless cooperative inference with fastest-host fallback. " +
  "Client for Windows, Linux, and macOS — all platforms released together.";

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: TITLE,
  description: DESCRIPTION,
  openGraph: {
    type: "website",
    url: SITE_URL,
    siteName: "ModelSwarm",
    title: TITLE,
    description: DESCRIPTION,
    images: [
      {
        url: "/hero.png",
        width: 1672,
        height: 941,
        alt: "ModelSwarm — a glowing network of peer nodes converging into a central verification tree above a laptop",
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: TITLE,
    description: DESCRIPTION,
    images: ["/hero.png"],
  },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
