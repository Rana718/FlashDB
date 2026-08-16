import Link from "next/link";
import Image from "next/image";
import { Lock, Zap, Cpu, RefreshCw, HardDrive, Radio } from "lucide-react";
import type { ReactNode } from "react";

function Bar({
   label,
   value,
   max,
   color,
}: {
   label: string;
   value: number;
   max: number;
   color: string;
}) {
   const pct = Math.round((value / max) * 100);
   return (
      <div className="flex items-center gap-3">
         <span className="w-28 text-xs font-medium text-muted shrink-0 text-right">
            {label}
         </span>
         <div className="flex-1 h-7 rounded-md bg-code-bg overflow-hidden relative">
            <div
               className={`h-full rounded-md ${color} transition-all duration-700`}
               style={{ width: `${pct}%` }}
            />
            <span className="absolute right-2 top-1/2 -translate-y-1/2 text-xs font-bold text-fg">
               {value >= 1000 ? `${(value / 1000).toFixed(1)}M` : `${value}K`}
            </span>
         </div>
      </div>
   );
}

function StatCard({
   value,
   unit,
   label,
   color,
}: {
   value: string;
   unit: string;
   label: string;
   color: string;
}) {
   return (
      <div className="rounded-xl border border-border bg-card/50 p-5 text-left backdrop-blur-sm">
         <p className={`text-3xl font-bold ${color}`}>
            {value}
            <span className="text-lg ml-0.5">{unit}</span>
         </p>
         <p className="text-sm text-muted mt-1.5">{label}</p>
      </div>
   );
}

function FeatureCard({
   icon,
   title,
   desc,
}: {
   icon: ReactNode;
   title: string;
   desc: string;
}) {
   return (
      <div className="rounded-xl border border-border bg-card/30 p-5 text-left hover:border-primary/40 transition-colors">
         <div className="text-primary">{icon}</div>
         <h3 className="text-sm font-semibold text-fg mt-3">{title}</h3>
         <p className="text-xs text-muted mt-1.5 leading-relaxed">{desc}</p>
      </div>
   );
}

export default function HomePage() {
   return (
      <main className="flex flex-1 flex-col items-center px-4 py-16">
         {/* Hero */}
         <div className="text-center max-w-3xl">
            <Image
               src="/logo.png"
               alt="FyroDB"
               width={72}
               height={72}
               className="mx-auto mb-6 rounded-xl"
            />
            <h1 className="text-5xl font-bold tracking-tight text-fg sm:text-6xl">
               Fyro<span className="text-primary">DB</span>
            </h1>
            <p className="mt-4 text-lg text-muted max-w-2xl mx-auto leading-relaxed">
               A Redis-compatible, lock-free in-memory key-value store written
               in Rust. A single node outperforms a 6-node Redis Cluster.
            </p>
            <div className="mt-8 flex flex-wrap gap-3 justify-center">
               <Link
                  href="/docs/getting-started"
                  className="rounded-lg bg-primary px-6 py-2.5 text-sm font-semibold text-primary-fg hover:opacity-90 transition-opacity shadow-lg shadow-primary/20"
               >
                  Get Started
               </Link>
               <Link
                  href="/docs/benchmarks"
                  className="rounded-lg border border-border px-6 py-2.5 text-sm font-medium text-fg hover:bg-card transition-colors"
               >
                  View Benchmarks
               </Link>
               <a
                  href="https://github.com/Rana718/FyroDB"
                  target="_blank"
                  rel="noopener"
                  className="rounded-lg border border-border px-6 py-2.5 text-sm font-medium text-fg hover:bg-card transition-colors"
               >
                  GitHub ↗
               </a>
            </div>
         </div>

         {/* Peak Stats */}
         <div className="mt-20 grid gap-4 sm:grid-cols-2 lg:grid-cols-4 max-w-5xl w-full">
            <StatCard
               value="14.9"
               unit="M"
               label="SET ops/sec (pipeline-100)"
               color="text-accent-green"
            />
            <StatCard
               value="19.3"
               unit="M"
               label="GET ops/sec (pipeline-100)"
               color="text-accent-blue"
            />
            <StatCard
               value="36.8"
               unit="M"
               label="Pub/Sub msg/sec"
               color="text-accent-purple"
            />
            <StatCard
               value="4.2"
               unit="x"
               label="faster than Redis Cluster"
               color="text-primary"
            />
         </div>

         {/* Benchmark Graphs */}
         <section className="mt-20 max-w-4xl w-full">
            <h2 className="text-2xl font-bold text-fg text-center mb-2">
               Benchmark Comparison
            </h2>
            <p className="text-sm text-muted text-center mb-10">
               SET ops/sec — pipeline-100, 100 clients, 1M ops. Higher is
               better.
            </p>

            <div className="rounded-xl border border-border bg-card/30 p-6 space-y-3">
               <Bar
                  label="FyroDB"
                  value={14900}
                  max={14900}
                  color="bg-primary"
               />
               <Bar
                  label="Redis Cluster"
                  value={7900}
                  max={14900}
                  color="bg-accent-blue"
               />
               <Bar
                  label="DragonflyDB"
                  value={3780}
                  max={14900}
                  color="bg-accent-purple"
               />
               <Bar
                  label="DiceDB"
                  value={1620}
                  max={14900}
                  color="bg-accent-yellow"
               />
            </div>
         </section>

         <section className="mt-12 max-w-4xl w-full">
            <p className="text-sm text-muted text-center mb-6">
               GET ops/sec — pipeline-100, 100 clients, 1M ops. Higher is
               better.
            </p>

            <div className="rounded-xl border border-border bg-card/30 p-6 space-y-3">
               <Bar
                  label="FyroDB"
                  value={19300}
                  max={19300}
                  color="bg-primary"
               />
               <Bar
                  label="Redis Cluster"
                  value={8300}
                  max={19300}
                  color="bg-accent-blue"
               />
               <Bar
                  label="DragonflyDB"
                  value={3970}
                  max={19300}
                  color="bg-accent-purple"
               />
               <Bar
                  label="DiceDB"
                  value={1880}
                  max={19300}
                  color="bg-accent-yellow"
               />
            </div>
         </section>

         <section className="mt-12 max-w-4xl w-full">
            <p className="text-sm text-muted text-center mb-6">
               Pub/Sub delivery — msg/sec. Higher is better.
            </p>

            <div className="rounded-xl border border-border bg-card/30 p-6 space-y-3">
               <Bar
                  label="FyroDB"
                  value={36800}
                  max={36800}
                  color="bg-primary"
               />
               <Bar
                  label="DragonflyDB"
                  value={12780}
                  max={36800}
                  color="bg-accent-purple"
               />
               <Bar
                  label="DiceDB"
                  value={8390}
                  max={36800}
                  color="bg-accent-yellow"
               />
               <Bar
                  label="Redis Cluster"
                  value={7300}
                  max={36800}
                  color="bg-accent-blue"
               />
            </div>
         </section>

         {/* Resource Usage */}
         <section className="mt-20 max-w-4xl w-full">
            <h2 className="text-2xl font-bold text-fg text-center mb-2">
               Resource Efficiency
            </h2>
            <p className="text-sm text-muted text-center mb-8">
               FyroDB (1 node) vs Redis Cluster (6 nodes) during benchmark
            </p>

            <div className="grid sm:grid-cols-2 gap-4">
               <div className="rounded-xl border border-border bg-card/30 p-5">
                  <p className="text-xs uppercase tracking-wider text-muted font-semibold mb-4">
                     FyroDB (1 node)
                  </p>
                  <div className="space-y-3">
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Idle RSS</span>
                        <span className="text-fg font-medium">~55 MB</span>
                     </div>
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Peak RSS</span>
                        <span className="text-fg font-medium">~235 MB</span>
                     </div>
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Peak CPU</span>
                        <span className="text-accent-green font-medium">
                           ~60%
                        </span>
                     </div>
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Avg CPU</span>
                        <span className="text-accent-green font-medium">
                           ~50%
                        </span>
                     </div>
                  </div>
               </div>
               <div className="rounded-xl border border-border bg-card/30 p-5">
                  <p className="text-xs uppercase tracking-wider text-muted font-semibold mb-4">
                     Redis Cluster (6 nodes)
                  </p>
                  <div className="space-y-3">
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Idle RSS</span>
                        <span className="text-fg font-medium">~75 MB</span>
                     </div>
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Peak RSS</span>
                        <span className="text-fg font-medium">~154 MB</span>
                     </div>
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Peak CPU</span>
                        <span className="text-accent-red font-medium">
                           ~96%
                        </span>
                     </div>
                     <div className="flex justify-between text-sm">
                        <span className="text-muted">Avg CPU</span>
                        <span className="text-accent-yellow font-medium">
                           ~25%
                        </span>
                     </div>
                  </div>
               </div>
            </div>
         </section>

         {/* Why FyroDB */}
         <section className="mt-20 max-w-5xl w-full">
            <h2 className="text-2xl font-bold text-fg text-center mb-2">
               Why FyroDB?
            </h2>
            <p className="text-sm text-muted text-center mb-8">
               Built for maximum throughput with minimum complexity
            </p>

            <div className="grid sm:grid-cols-2 lg:grid-cols-3 gap-4">
               <FeatureCard
                  icon={<Lock size={22} />}
                  title="Fully Lock-Free"
                  desc="No mutex on the data path. Epoch-based reclamation frees values only after all readers unpin."
               />
               <FeatureCard
                  icon={<Zap size={22} />}
                  title="Zero-Copy GET"
                  desc="Reads write directly from stored value to TCP buffer. No String clone, no allocation."
               />
               <FeatureCard
                  icon={<Cpu size={22} />}
                  title="Thread-per-Core"
                  desc="One epoll loop per CPU core with SO_REUSEPORT for kernel-level connection distribution."
               />
               <FeatureCard
                  icon={<RefreshCw size={22} />}
                  title="Redis Compatible"
                  desc="Speaks RESP protocol. Any Redis client, SDK, or CLI tool works without modification."
               />
               <FeatureCard
                  icon={<HardDrive size={22} />}
                  title="RDB Persistence"
                  desc="Automatic snapshots every 5 minutes, atomic writes, crash-safe. Same model as Redis."
               />
               <FeatureCard
                  icon={<Radio size={22} />}
                  title="Lock-Free Pub/Sub"
                  desc="36.8M msg/sec delivery using Arc-cloned snapshots. No locks, no use-after-free risk."
               />
            </div>
         </section>

         {/* Quick Start */}
         <section className="mt-20 max-w-2xl w-full">
            <h2 className="text-2xl font-bold text-fg text-center mb-6">
               Try It Now
            </h2>
            <pre className="rounded-xl border border-border bg-code-bg px-6 py-5 text-left text-sm font-mono text-fg overflow-x-auto">
               {`docker run -p 8000:8000 rana718/fyrodb:latest

redis-cli -p 8000
127.0.0.1:8000> SET hello world
OK
127.0.0.1:8000> GET hello
"world"
127.0.0.1:8000> SUBSCRIBE events
Reading messages...`}
            </pre>
         </section>

         {/* Footer CTA */}
         <section className="mt-20 mb-10 text-center">
            <p className="text-muted text-sm">
               6-core Intel i5-11400H • 100 clients • 1M operations •{" "}
               <Link
                  href="/docs/benchmarks"
                  className="text-primary hover:underline"
               >
                  full benchmark details →
               </Link>
            </p>
         </section>
      </main>
   );
}
