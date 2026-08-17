import type { MetadataRoute } from "next";
import { getAllSlugs } from "@/lib/docs";

const BASE_URL = "https://fyrodb.vercel.app";

export default function sitemap(): MetadataRoute.Sitemap {
   const slugs = getAllSlugs();

   const docRoutes: MetadataRoute.Sitemap = slugs.map((slug) => ({
      url: `${BASE_URL}/docs/${slug}`,
      lastModified: new Date(),
      changeFrequency: "weekly",
      priority: 0.8,
   }));

   return [
      {
         url: BASE_URL,
         lastModified: new Date(),
         changeFrequency: "weekly",
         priority: 1.0,
      },
      ...docRoutes,
   ];
}
