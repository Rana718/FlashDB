import { MDXRemote } from "next-mdx-remote/rsc";
import remarkGfm from "remark-gfm";

export function MDXContent({ source }: { source: string }) {
   return (
      <div className="prose">
         <MDXRemote
            source={source}
            options={{ mdxOptions: { remarkPlugins: [remarkGfm] } }}
         />
      </div>
   );
}
