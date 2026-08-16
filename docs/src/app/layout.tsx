import type { Metadata } from "next";
import { ThemeProvider } from "@/components/ThemeProvider";
import { Navbar } from "@/components/Navbar";
import { getAllDocs } from "@/lib/docs";
import "./globals.css";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";

export const metadata: Metadata = {
   title: { default: "FyroDB Documentation", template: "%s — FyroDB Docs" },
   description:
      "Documentation for FyroDB — a Redis-compatible lock-free in-memory key-value store written in Rust.",
   keywords: [
      "FyroDB",
      "Redis",
      "Rust",
      "key-value store",
      "lock-free",
      "in-memory database",
   ],
   metadataBase: new URL("https://fyrodb.dev"),
   openGraph: {
      type: "website",
      title: "FyroDB Documentation",
      description:
         "Redis-compatible lock-free in-memory key-value store written in Rust. 14M+ ops/sec on a single node.",
      images: [{ url: "/logo.png", width: 512, height: 512 }],
   },
   twitter: {
      card: "summary",
      title: "FyroDB Docs",
      description: "Lock-free Redis alternative in Rust",
   },
   icons: { icon: "/logo.png" },
};

export default function RootLayout({
   children,
}: {
   children: React.ReactNode;
}) {
   const docs = getAllDocs();
   return (
      <html lang="en" suppressHydrationWarning className="font-sans">
         <body className="min-h-screen flex flex-col">
            <ThemeProvider>
               <Navbar docs={docs} />
               {children}
            </ThemeProvider>
         </body>
      </html>
   );
}
