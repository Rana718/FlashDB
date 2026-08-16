import { Sidebar } from "@/components/Sidebar";
import { Search } from "@/components/Search";
import { getAllDocs } from "@/lib/docs";

export default function DocsLayout({
   children,
}: {
   children: React.ReactNode;
}) {
   const docs = getAllDocs();

   return (
      <div className="flex flex-1">
         <Sidebar />
         <div className="flex-1 min-w-0">
            <div className="border-b border-border px-6 py-3 lg:hidden">
               <Search docs={docs} />
            </div>
            <div className="mx-auto max-w-3xl px-6 py-10">
               <div className="hidden lg:block mb-6">
                  <Search docs={docs} />
               </div>
               {children}
            </div>
         </div>
      </div>
   );
}
