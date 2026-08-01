import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";

/// 监听后端 usage-log-recorded 事件，收到后失效统计查询缓存
export function useUsageEventBridge() {
  const queryClient = useQueryClient();

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    listen("usage-log-recorded", () => {
      void Promise.all([
        queryClient.invalidateQueries({ queryKey: ["usage", "summary"] }),
        queryClient.invalidateQueries({ queryKey: ["usage", "trends"] }),
        queryClient.invalidateQueries({ queryKey: ["usage", "providers"] }),
        queryClient.invalidateQueries({ queryKey: ["usage", "models"] }),
        queryClient.invalidateQueries({ queryKey: ["usage", "logs"] }),
      ]);
    })
      .then((un) => {
        unlisten = un;
      })
      .catch((error) => {
        console.error("无法监听使用统计更新事件", error);
      });

    return () => {
      unlisten?.();
    };
  }, [queryClient]);
}
