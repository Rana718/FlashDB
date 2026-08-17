import type { NextConfig } from "next";

const nextConfig: NextConfig = {
   images: {
      unoptimized: true,
   },

   reactStrictMode: true,
   compress: true,
   typescript: {
      ignoreBuildErrors: false,
   },

   async headers() {
      return [
         {
            source: "/(.*)",
            headers: [
               { key: "X-Content-Type-Options", value: "nosniff" },
               { key: "X-Frame-Options", value: "DENY" },
               { key: "X-XSS-Protection", value: "1; mode=block" },
               {
                  key: "Referrer-Policy",
                  value: "strict-origin-when-cross-origin",
               },
            ],
         },
         {
            source: "/fonts/(.*)",
            headers: [
               {
                  key: "Cache-Control",
                  value: "public, max-age=31536000, immutable",
               },
            ],
         },
         {
            source: "/logo.png",
            headers: [
               {
                  key: "Cache-Control",
                  value: "public, max-age=86400, stale-while-revalidate=604800",
               },
            ],
         },
         {
            source: "/sitemap.xml",
            headers: [
               {
                  key: "Cache-Control",
                  value: "public, max-age=3600, stale-while-revalidate=86400",
               },
            ],
         },
      ];
   },
};

export default nextConfig;
