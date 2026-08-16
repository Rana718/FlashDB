"use client";

import Image from "next/image";
import Link from "next/link";
import { useTheme } from "next-themes";
import { useEffect, useMemo, useState } from "react";
import { FiArrowUpRight, FiMoon, FiSearch, FiSun } from "react-icons/fi";
import { FaGithub } from "react-icons/fa";
import type { DocMeta } from "@/lib/docs";

const navLinks = [
   { href: "/docs/getting-started", label: "Get started" },
   { href: "/docs/commands-strings", label: "Commands" },
   { href: "/docs/benchmarks", label: "Benchmarks" },
];

export function Navbar({ docs }: { docs: DocMeta[] }) {
   const { theme, setTheme } = useTheme();
   const [mounted, setMounted] = useState(false);
   const [searchOpen, setSearchOpen] = useState(false);
   const [query, setQuery] = useState("");
   const [stars, setStars] = useState<string | null>(null);

   const results = useMemo(() => {
      const value = query.trim().toLowerCase();
      if (!value) return docs;
      return docs.filter((doc) =>
         `${doc.title} ${doc.description} ${doc.slug}`
            .toLowerCase()
            .includes(value),
      );
   }, [docs, query]);

   useEffect(() => setMounted(true), []);
   useEffect(() => {
      const onKey = (event: KeyboardEvent) => {
         if (
            (event.metaKey || event.ctrlKey) &&
            event.key.toLowerCase() === "k"
         ) {
            event.preventDefault();
            setSearchOpen((open) => !open);
         }
         if (event.key === "Escape") setSearchOpen(false);
      };
      window.addEventListener("keydown", onKey);
      fetch("https://api.github.com/repos/Rana718/FyroDB")
         .then((response) => (response.ok ? response.json() : null))
         .then(
            (data) =>
               data?.stargazers_count != null &&
               setStars(
                  new Intl.NumberFormat("en", { notation: "compact" }).format(
                     data.stargazers_count,
                  ),
               ),
         )
         .catch(() => undefined);
      return () => window.removeEventListener("keydown", onKey);
   }, []);

   const closeSearch = () => {
      setSearchOpen(false);
      setQuery("");
   };

   return (
      <>
         <header className="sticky top-0 z-50 border-b border-border/80 bg-bg/85 backdrop-blur-xl">
            <nav className="mx-auto flex h-16 max-w-[1500px] items-center gap-8 px-5 lg:px-8">
               <Link
                  href="/"
                  className="flex items-center gap-2.5 font-semibold text-fg"
               >
                  <Image
                     src="/logo.png"
                     alt="FyroDB"
                     width={34}
                     height={34}
                     className="object-contain"
                  />
                  <span>
                     Fyro<span className="text-primary">DB</span>
                  </span>
               </Link>
               <div className="hidden items-center gap-6 text-sm md:flex">
                  {navLinks.map((link) => (
                     <Link
                        key={link.href}
                        href={link.href}
                        className="text-muted transition-colors hover:text-fg"
                     >
                        {link.label}
                     </Link>
                  ))}
               </div>
               <div className="ml-auto flex items-center gap-2">
                  <button
                     onClick={() => setSearchOpen(true)}
                     className="hidden h-9 items-center gap-2 rounded-lg border border-border bg-card/70 px-3 text-sm text-muted transition-colors hover:border-primary/50 hover:text-fg sm:flex"
                     aria-label="Search all documentation"
                  >
                     <FiSearch />
                     <span>Search docs</span>
                     <kbd className="ml-3 rounded border border-border px-1.5 py-0.5 text-[10px]">
                        ⌘ K
                     </kbd>
                  </button>
                  <a
                     href="https://github.com/Rana718/FyroDB"
                     target="_blank"
                     rel="noopener"
                     className="flex h-9 items-center gap-1.5 rounded-lg px-2 text-muted transition-colors hover:bg-card hover:text-fg"
                     aria-label="FyroDB GitHub repository"
                  >
                     <FaGithub size={17} />
                     <span className="hidden text-xs sm:inline">
                        {stars ? `${stars} stars` : "GitHub"}
                     </span>
                     <FiArrowUpRight size={13} />
                  </a>
                  {mounted && (
                     <button
                        onClick={() =>
                           setTheme(theme === "dark" ? "light" : "dark")
                        }
                        className="rounded-lg p-2 text-muted transition-colors hover:bg-card hover:text-fg"
                        aria-label="Toggle theme"
                     >
                        {theme === "dark" ? <FiSun /> : <FiMoon />}
                     </button>
                  )}
               </div>
            </nav>
         </header>
         {searchOpen && (
            <div
               className="fixed inset-0 z-[60] bg-black/50 p-4 pt-[10vh] backdrop-blur-sm"
               onMouseDown={closeSearch}
            >
               <div
                  className="mx-auto max-w-2xl overflow-hidden rounded-xl border border-border bg-bg shadow-2xl"
                  onMouseDown={(event) => event.stopPropagation()}
               >
                  <div className="flex items-center gap-3 border-b border-border px-5 py-4">
                     <FiSearch className="text-muted" />
                     <input
                        autoFocus
                        value={query}
                        onChange={(event) => setQuery(event.target.value)}
                        placeholder="Search every documentation page..."
                        className="flex-1 bg-transparent text-base text-fg outline-none placeholder:text-muted"
                     />
                     <kbd className="rounded border border-border px-2 py-1 text-xs text-muted">
                        ESC
                     </kbd>
                  </div>
                  <div className="max-h-[55vh] overflow-y-auto p-2">
                     {results.length ? (
                        results.map((doc) => (
                           <Link
                              key={doc.slug}
                              href={`/docs/${doc.slug}`}
                              onClick={closeSearch}
                              className="group flex items-center justify-between gap-6 rounded-lg px-3 py-3 hover:bg-card"
                           >
                              <span>
                                 <span className="block text-sm font-medium text-fg group-hover:text-primary">
                                    {doc.title}
                                 </span>
                                 <span className="mt-0.5 block text-xs text-muted">
                                    {doc.description}
                                 </span>
                              </span>
                              <FiArrowUpRight className="shrink-0 text-muted" />
                           </Link>
                        ))
                     ) : (
                        <p className="px-3 py-8 text-center text-sm text-muted">
                           No documentation matched “{query}”.
                        </p>
                     )}
                  </div>
               </div>
            </div>
         )}
      </>
   );
}
