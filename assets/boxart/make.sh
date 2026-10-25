#!/bin/bash
# Builds the front cover SVG and PNG for the home shell Demo Disc case (square, 120 mm insert).
# Needs a fonts.conf in this directory that lists a folder holding Michroma and Jost (both SIL OFL).
cd "$(dirname "$0")"
CX=600; CY=575; R=235; RH=58; RL=132
sectors=$(awk -v cx=$CX -v cy=$CY -v r=$R -v rh=$RH -v n=90 'BEGIN{
  pi=3.14159265358979;
  for(k=0;k<n;k++){
    a0=2*pi*k/n; a1=2*pi*(k+1)/n; am=(a0+a1)/2;
    s=cos(2*(am-0.9)); s=(s+1)/2; s=s*s*s;           # two sheen lobes
    t=cos(2*(am-0.9+pi/2)); t=(t+1)/2; t=t*t*t*0.35;  # a faint cross lobe
    l=s+t; if(l>1)l=1;
    R_=int(18+l*120); G_=int(34+l*170); B_=int(44+l*196);
    printf "<path d=\"M%.2f %.2f L%.2f %.2f L%.2f %.2f L%.2f %.2f Z\" fill=\"rgb(%d,%d,%d)\" stroke=\"rgb(%d,%d,%d)\" stroke-width=\"1.2\"/>\n",
      cx+r*sin(a0), cy-r*cos(a0), cx+r*sin(a1), cy-r*cos(a1), cx+rh*sin(a1), cy-rh*cos(a1), cx+rh*sin(a0), cy-rh*cos(a0), R_,G_,B_, R_,G_,B_;
  }
}')
rings=$(awk -v cx=$CX -v cy=$CY 'BEGIN{ for(r=60;r<=1000;r+=64) printf "<circle cx=\"%d\" cy=\"%d\" r=\"%d\" fill=\"none\" stroke=\"#22c8e5\" stroke-opacity=\"%.3f\" stroke-width=\"1.4\"/>\n", cx, cy, r, 0.16*exp(-r/700); }')
stars=$(awk 'BEGIN{ srand(7); for(i=0;i<110;i++){ x=rand()*1200; y=rand()*1200; r=0.8+rand()*1.6; o=0.15+rand()*0.5; printf "<circle cx=\"%.1f\" cy=\"%.1f\" r=\"%.1f\" fill=\"#cfeaf3\" fill-opacity=\"%.2f\"/>\n", x,y,r,o; } }')
# Seven cover tiles, one per program in menu order.
tiles=""
names=(breakout celeste nitroxide-aerial voxide-day quake-gameplay cortex-current-gameplay psxcel-chart)
w=132; h=99; gap=14; total=$((7*w+6*gap)); x0=$(( (1200-total)/2 )); y=1015
for i in 0 1 2 3 4 5 6; do
  x=$((x0+i*(w+gap)))
  tiles+="<g><rect x=\"$((x-5))\" y=\"$((y-5))\" width=\"$((w+10))\" height=\"$((h+10))\" fill=\"#07121a\" stroke=\"#6c7f8f\" stroke-width=\"1.6\"/>
<image href=\"shots/${names[$i]}.png\" x=\"$x\" y=\"$y\" width=\"$w\" height=\"$h\" preserveAspectRatio=\"xMidYMid slice\"/>
<rect x=\"$x\" y=\"$y\" width=\"$w\" height=\"$h\" fill=\"url(#gloss)\"/></g>
"
done
LOGO=$(perl -0ne 'print "<g transform=\"translate(212 33) scale(0.78)\">" . (/(<g transform=.*?<\/g>)/s ? $1 : "") . "<\/g>"' psoxide-logo.svg)
LOGODEFS=$(perl -0ne 'print $1 if /(<defs>.*?<\/defs>)/s' psoxide-logo.svg)
cat >| cover.svg <<SVG
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="120mm" height="120mm" viewBox="0 0 1200 1200" role="img" aria-label="PSoXide Demo Disc front cover">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0b1a24"/><stop offset="1" stop-color="#04080c"/></linearGradient>
    <radialGradient id="glow" cx="50%" cy="51%" r="52%"><stop offset="0" stop-color="#22c8e5" stop-opacity=".30"/><stop offset=".55" stop-color="#22c8e5" stop-opacity=".07"/><stop offset="1" stop-color="#22c8e5" stop-opacity="0"/></radialGradient>
    <radialGradient id="halo" cx="50%" cy="50%" r="50%"><stop offset=".62" stop-color="#22c8e5" stop-opacity=".0"/><stop offset=".8" stop-color="#22c8e5" stop-opacity=".38"/><stop offset="1" stop-color="#22c8e5" stop-opacity="0"/></radialGradient>
    <linearGradient id="gloss" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".14"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient>
    <linearGradient id="spine" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#6c7f8f" stop-opacity=".55"/><stop offset="1" stop-color="#6c7f8f" stop-opacity="0"/></linearGradient>
    $LOGODEFS
    <clipPath id="face"><rect width="1200" height="1200"/></clipPath>
  </defs>
  <g clip-path="url(#face)">
    <rect width="1200" height="1200" fill="url(#bg)"/>
    <rect width="1200" height="1200" fill="url(#glow)"/>
    $rings
    $stars
    <!-- the disc -->
    <circle cx="$CX" cy="$CY" r="$((R+130))" fill="url(#halo)"/>
    <circle cx="$CX" cy="$CY" r="$((R+16))" fill="none" stroke="#22c8e5" stroke-opacity=".35" stroke-width="2"/>
    $sectors
    <circle cx="$CX" cy="$CY" r="$R" fill="none" stroke="#9fe9f7" stroke-opacity=".75" stroke-width="3"/>
    <circle cx="$CX" cy="$CY" r="$RL" fill="none" stroke="#22c8e5" stroke-width="3"/>
    <circle cx="$CX" cy="$CY" r="$((RL-14))" fill="none" stroke="#22c8e5" stroke-opacity=".35" stroke-width="2"/>
    <circle cx="$CX" cy="$CY" r="$RH" fill="#03070b"/>
    <circle cx="$CX" cy="$CY" r="$((RH+7))" fill="none" stroke="#22c8e5" stroke-opacity=".6" stroke-width="2"/>
    <g stroke="#22c8e5" stroke-width="7" stroke-linecap="round"><line x1="$((CX-30))" y1="$((CY-30))" x2="$((CX+30))" y2="$((CY+30))"/><line x1="$((CX+30))" y1="$((CY-30))" x2="$((CX-30))" y2="$((CY+30))"/></g>
    <!-- wordmark -->
    $LOGO
    <text x="600" y="228" text-anchor="middle" font-family="Jost" font-weight="300" font-size="31" letter-spacing="6" fill="#9db0be">HOMEBREW FOR THE PLAYSTATION</text>
    <!-- title -->
    <text x="600" y="905" text-anchor="middle" font-family="Michroma" font-size="76" letter-spacing="9" fill="#f4f8fb">DEMO DISC</text>
    <text x="600" y="954" text-anchor="middle" font-family="Jost" font-weight="400" font-size="28" letter-spacing="9" fill="#22c8e5">PLAYSTATION HOMEBREW · 2026</text>
    $tiles
    <!-- case spine highlight -->
    <rect x="0" y="0" width="26" height="1200" fill="url(#spine)"/>
    <rect x="1" y="1" width="1198" height="1198" fill="none" stroke="#6c7f8f" stroke-opacity=".6" stroke-width="2"/>
  </g>
</svg>
SVG
FONTCONFIG_FILE=$PWD/fonts.conf rsvg-convert -w 1800 -h 1800 cover.svg -o cover.png
ls -la cover.svg cover.png
