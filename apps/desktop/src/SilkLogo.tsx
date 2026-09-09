import React from "react";

export type SilkLogoProps = {
  size?: number | string;
  className?: string;
  style?: React.CSSProperties;
  color?: string;
};

export function SilkLogo({
  size = 20,
  className = "",
  style,
  color = "currentColor",
}: SilkLogoProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill={color}
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      style={{ display: "inline-block", verticalAlign: "middle", flexShrink: 0, ...style }}
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      <rect x="12" y="3" width="8" height="1" />
      <rect x="10" y="4" width="12" height="1" />
      <rect x="8" y="5" width="16" height="1" />
      <rect x="7" y="6" width="8" height="1" />
      <rect x="17" y="6" width="8" height="1" />
      <rect x="6" y="7" width="6" height="1" />
      <rect x="20" y="7" width="6" height="1" />
      <rect x="6" y="8" width="4" height="1" />
      <rect x="22" y="8" width="5" height="1" />
      <rect x="7" y="9" width="2" height="1" />
      <rect x="23" y="9" width="2" height="1" />
      <rect x="19" y="10" width="11" height="1" />
      <rect x="19" y="11" width="11" height="1" />
      <rect x="20" y="12" width="9" height="1" />
      <rect x="21" y="13" width="7" height="1" />
      <rect x="22" y="14" width="5" height="1" />
      <rect x="23" y="15" width="2" height="1" />
      <rect x="7" y="16" width="2" height="1" />
      <rect x="5" y="17" width="5" height="1" />
      <rect x="4" y="18" width="7" height="1" />
      <rect x="3" y="19" width="9" height="1" />
      <rect x="2" y="20" width="11" height="1" />
      <rect x="2" y="21" width="11" height="1" />
      <rect x="7" y="22" width="2" height="1" />
      <rect x="23" y="22" width="2" height="1" />
      <rect x="5" y="23" width="5" height="1" />
      <rect x="22" y="23" width="4" height="1" />
      <rect x="6" y="24" width="6" height="1" />
      <rect x="20" y="24" width="6" height="1" />
      <rect x="7" y="25" width="8" height="1" />
      <rect x="17" y="25" width="8" height="1" />
      <rect x="8" y="26" width="16" height="1" />
      <rect x="10" y="27" width="12" height="1" />
      <rect x="12" y="28" width="8" height="1" />
    </svg>
  );
}

export default SilkLogo;
