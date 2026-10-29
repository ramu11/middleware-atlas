import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Middleware Atlas — Diagnostic Agent",
  description:
    "Describe a middleware incident, attach diagnostic evidence, and get a local-first investigation.",
};


export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>{children}
</body>
    </html>
  );
}
