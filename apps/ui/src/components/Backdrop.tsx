/** The sky scene behind the glass: two blurred glows, a sun and two waves, as in the design. */
export function Backdrop() {
  return (
    <div className="backdrop" aria-hidden="true">
      <i className="backdrop-glow backdrop-glow--mint" />
      <i className="backdrop-glow backdrop-glow--peach" />
      <i className="backdrop-sun" />
      <svg viewBox="0 0 1440 420" preserveAspectRatio="none">
        <path d="M0 120 C220 40 420 40 640 110 S1060 190 1440 90 V420 H0Z" fill="#fff" opacity=".28" />
        <path d="M0 230 C240 150 440 170 680 230 S1080 300 1440 210 V420 H0Z" fill="#fff" opacity=".5" />
      </svg>
    </div>
  );
}
