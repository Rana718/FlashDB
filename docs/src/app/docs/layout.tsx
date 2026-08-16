import { Sidebar } from "@/components/Sidebar";

export default function DocsLayout({
   children,
}: {
   children: React.ReactNode;
}) {
   return (
      <div className="flex flex-1 bg-bg">
         <Sidebar />
         <div className="flex-1 min-w-0">
            <div className="w-full max-w-[1180px] px-6 py-10 lg:px-12 lg:py-14 xl:px-16">
               {children}
            </div>
         </div>
      </div>
   );
}
