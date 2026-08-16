"use client";
import Link from "next/link";
import Image from "next/image";
import { useTheme } from "next-themes";
import { useEffect, useState } from "react";
import { Sun, Moon } from "lucide-react";

function GithubIcon({ size = 18 }: { size?: number }) {
   return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
         <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z" />
      </svg>
   );
}

export function Navbar() {
   const { theme, setTheme } = useTheme();
   const [mounted, setMounted] = useState(false);
   useEffect(() => setMounted(true), []);

   return (
      <header className="sticky top-0 z-50 border-b border-border bg-bg/80 backdrop-blur-md">
         <nav className="mx-auto flex h-14 max-w-6xl items-center justify-between px-4">
            <Link
               href="/"
               className="flex items-center gap-2 font-semibold text-fg"
            >
               <Image
                  src="/logo.png"
                  alt="FyroDB"
                  width={28}
                  height={28}
                  className="rounded"
               />
               <span>FyroDB</span>
            </Link>
            <div className="flex items-center gap-4 text-sm">
               <Link
                  href="/docs/getting-started"
                  className="text-muted hover:text-fg transition-colors"
               >
                  Docs
               </Link>
               <Link
                  href="/docs/benchmarks"
                  className="text-muted hover:text-fg transition-colors"
               >
                  Benchmarks
               </Link>
               <a
                  href="https://github.com/Rana718/FyroDB"
                  target="_blank"
                  rel="noopener"
                  className="text-muted hover:text-fg transition-colors"
               >
                  <GithubIcon size={18} />
               </a>
               {mounted && (
                  <button
                     onClick={() =>
                        setTheme(theme === "dark" ? "light" : "dark")
                     }
                     className="rounded-md p-1.5 text-muted hover:bg-card hover:text-fg transition-colors"
                     aria-label="Toggle theme"
                  >
                     {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
                  </button>
               )}
            </div>
         </nav>
      </header>
   );
}
