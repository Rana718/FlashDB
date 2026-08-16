import { notFound } from "next/navigation";
import { getDoc, getAllSlugs } from "@/lib/docs";
import { MDXContent } from "@/components/MDXContent";
import type { Metadata } from "next";

interface Props {
   params: Promise<{ slug: string }>;
}

export async function generateStaticParams() {
   return getAllSlugs().map((slug) => ({ slug }));
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
   const { slug } = await params;
   const doc = getDoc(slug);
   if (!doc) return {};
   return { title: doc.title, description: doc.description };
}

export default async function DocPage({ params }: Props) {
   const { slug } = await params;
   const doc = getDoc(slug);
   if (!doc) notFound();

   return (
      <article>
         <h1 className="text-3xl font-bold text-fg mb-2">{doc.title}</h1>
         <p className="text-muted mb-8">{doc.description}</p>
         <MDXContent source={doc.content} />
      </article>
   );
}
