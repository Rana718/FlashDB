import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import { ThemeProvider } from "@/components/ThemeProvider";
import { Navbar } from "@/components/Navbar";
import "./globals.css";

const geistSans = Geist({ variable: "--font-geist-sans", subsets: ["latin"] });
const geistMono = Geist_Mono({
   variable: "--font-geist-mono",
   subsets: ["latin"],
});

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
   return (
      <html
         lang="en"
         suppressHydrationWarning
         className={`${geistSans.variable} ${geistMono.variable}`}
      >
         <body className="min-h-screen flex flex-col">
            <ThemeProvider>
               <Navbar />
               {children}
            </ThemeProvider>
         </body>
      </html>
   );
}
