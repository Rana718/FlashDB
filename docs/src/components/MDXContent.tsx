import { MDXRemote } from "next-mdx-remote/rsc";
import remarkGfm from "remark-gfm";
import rehypePrettyCode from "rehype-pretty-code";

export function MDXContent({ source }: { source: string }) {
   return (
      <div className="prose">
         <MDXRemote
            source={source}
            options={{
               mdxOptions: {
                  remarkPlugins: [remarkGfm],
                  rehypePlugins: [
                     [
                        rehypePrettyCode,
                        { theme: "github-dark-default", keepBackground: false },
                     ],
                  ],
               },
            }}
         />
      </div>
   );
}
