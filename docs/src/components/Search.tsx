"use client";
import { useState, useMemo } from "react";
import Link from "next/link";
import type { DocMeta } from "@/lib/docs";

export function Search({ docs }: { docs: DocMeta[] }) {
   const [query, setQuery] = useState("");
   const [open, setOpen] = useState(false);

   const results = useMemo(() => {
      if (!query.trim()) return [];
      const q = query.toLowerCase();
      return docs
         .filter(
            (d) =>
               d.title.toLowerCase().includes(q) ||
               d.description.toLowerCase().includes(q),
         )
         .slice(0, 8);
   }, [query, docs]);

   return (
      <div className="relative">
         <input
            type="text"
            placeholder="Search docs..."
            value={query}
            onChange={(e) => {
               setQuery(e.target.value);
               setOpen(true);
            }}
            onFocus={() => setOpen(true)}
            onBlur={() => setTimeout(() => setOpen(false), 150)}
            className="w-full rounded-md border border-border bg-card px-3 py-1.5 text-sm text-fg placeholder:text-muted focus:outline-none focus:ring-1 focus:ring-primary"
         />
         {open && results.length > 0 && (
            <div className="absolute top-full left-0 right-0 mt-1 rounded-md border border-border bg-bg shadow-lg z-50 max-h-64 overflow-y-auto">
               {results.map((doc) => (
                  <Link
                     key={doc.slug}
                     href={`/docs/${doc.slug}`}
                     className="block px-3 py-2 text-sm hover:bg-card border-b border-border last:border-0"
                     onClick={() => {
                        setOpen(false);
                        setQuery("");
                     }}
                  >
                     <span className="font-medium text-fg">{doc.title}</span>
                     <span className="block text-xs text-muted truncate">
                        {doc.description}
                     </span>
                  </Link>
               ))}
            </div>
         )}
      </div>
   );
}
