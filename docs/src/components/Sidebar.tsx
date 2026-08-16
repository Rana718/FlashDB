"use client";
import Link from "next/link";
import { usePathname } from "next/navigation";

const sections = [
   {
      title: "Getting Started",
      items: [
         { label: "Introduction", slug: "introduction" },
         { label: "Installation", slug: "getting-started" },
         { label: "Docker", slug: "docker" },
         { label: "Configuration", slug: "configuration" },
      ],
   },
   {
      title: "Usage",
      items: [
         { label: "CLI & Redis Clients", slug: "cli-usage" },
         { label: "Node.js (ioredis)", slug: "nodejs" },
         { label: "Python (redis-py)", slug: "python" },
         { label: "Go (go-redis)", slug: "golang" },
         { label: "Rust (redis-rs)", slug: "rust-client" },
      ],
   },
   {
      title: "Commands",
      items: [
         { label: "Strings", slug: "commands-strings" },
         { label: "Keys", slug: "commands-keys" },
         { label: "Hashes", slug: "commands-hashes" },
         { label: "Pub/Sub", slug: "commands-pubsub" },
         { label: "Server", slug: "commands-server" },
      ],
   },
   {
      title: "Architecture",
      items: [
         { label: "Overview", slug: "architecture" },
         { label: "Persistence", slug: "persistence" },
         { label: "Benchmarks", slug: "benchmarks" },
      ],
   },
];

export function Sidebar() {
   const pathname = usePathname();

   return (
      <aside className="hidden lg:block w-64 shrink-0 border-r border-border bg-sidebar-bg overflow-y-auto h-[calc(100vh-3.5rem)] sticky top-14 p-4">
         {sections.map((section) => (
            <div key={section.title} className="mb-5">
               <p className="text-xs font-semibold uppercase tracking-wider text-muted mb-2">
                  {section.title}
               </p>
               <ul className="space-y-0.5">
                  {section.items.map((item) => {
                     const href = `/docs/${item.slug}`;
                     const active = pathname === href;
                     return (
                        <li key={item.slug}>
                           <Link
                              href={href}
                              className={`block rounded-md px-3 py-1.5 text-sm transition-colors ${active ? "bg-primary/10 text-primary font-medium" : "text-muted hover:text-fg hover:bg-card"}`}
                           >
                              {item.label}
                           </Link>
                        </li>
                     );
                  })}
               </ul>
            </div>
         ))}
      </aside>
   );
}
