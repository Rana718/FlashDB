import fs from "fs";
import path from "path";
import matter from "gray-matter";

const CONTENT_DIR = path.join(process.cwd(), "src/content");

export interface DocMeta {
   slug: string;
   title: string;
   description: string;
   order: number;
}

export interface Doc extends DocMeta {
   content: string;
}

export function getAllDocs(): DocMeta[] {
   const files = fs.readdirSync(CONTENT_DIR).filter((f) => f.endsWith(".mdx"));
   return files
      .map((file) => {
         const raw = fs.readFileSync(path.join(CONTENT_DIR, file), "utf-8");
         const { data } = matter(raw);
         return {
            slug: file.replace(/\.mdx$/, ""),
            title: data.title || "",
            description: data.description || "",
            order: data.order || 99,
         };
      })
      .sort((a, b) => a.order - b.order);
}

export function getDoc(slug: string): Doc | null {
   const filePath = path.join(CONTENT_DIR, `${slug}.mdx`);
   if (!fs.existsSync(filePath)) return null;
   const raw = fs.readFileSync(filePath, "utf-8");
   const { data, content } = matter(raw);
   return {
      slug,
      title: data.title || "",
      description: data.description || "",
      order: data.order || 99,
      content,
   };
}

export function getAllSlugs(): string[] {
   return fs
      .readdirSync(CONTENT_DIR)
      .filter((f) => f.endsWith(".mdx"))
      .map((f) => f.replace(/\.mdx$/, ""));
}
