import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Middleware Atlas — Support Engineering Workspace",
  description:
    "Investigate middleware incidents with structured evidence and diagnostic tools.",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
