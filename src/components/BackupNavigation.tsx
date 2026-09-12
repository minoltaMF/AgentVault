import { Link } from "react-router-dom";
import { Button } from "@/components/ui/button";

export function BackupNavigation({ snapshots = false }: { snapshots?: boolean }) {
  return <nav aria-label="备份分类" className="flex flex-wrap gap-2 border-b px-6 py-3">
    <Button asChild variant={snapshots ? "ghost" : "secondary"} size="sm">
      <Link to="/codex/backups" aria-current={!snapshots ? "page" : undefined}>普通备份</Link>
    </Button>
    <Button asChild variant={snapshots ? "secondary" : "ghost"} size="sm">
      <Link to="/codex/delete-snapshots" aria-current={snapshots ? "page" : undefined}>删除前快照</Link>
    </Button>
  </nav>;
}
