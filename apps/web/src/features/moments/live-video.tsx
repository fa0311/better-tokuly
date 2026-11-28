import { type Ref, useEffect, useRef } from "react";
import { cn } from "#/lib/cn";

/** A camera's preview. `videoRef` is what a shot is taken from. */
export function LiveVideo({
  stream,
  mirror,
  videoRef,
  onLive,
  className,
  testId,
}: {
  stream: MediaStream;
  mirror: boolean;
  videoRef?: Ref<HTMLVideoElement>;
  onLive?(): void;
  className?: string;
  testId?: string;
}) {
  const own = useRef<HTMLVideoElement | null>(null);
  useEffect(() => {
    if (own.current) own.current.srcObject = stream;
  }, [stream]);
  return (
    <video
      ref={(element) => {
        own.current = element;
        if (typeof videoRef === "function") videoRef(element);
        else if (videoRef) videoRef.current = element;
      }}
      autoPlay
      muted
      playsInline
      onPlaying={onLive}
      className={cn("object-cover", mirror && "-scale-x-100", className)}
      data-testid={testId}
    />
  );
}
