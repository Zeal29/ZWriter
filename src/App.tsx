import Settings from "./Settings";
import Review from "./Review";
import "./App.css";

function App() {
  const isSettings = window.location.hash.includes("/settings");
  return isSettings ? <Settings /> : <Review />;
}

export default App;
