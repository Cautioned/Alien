import express from 'express';

const app = express();
const port = 3000; // You can change this to any port you prefer

// Middleware to parse JSON requests
app.use(express.json());

// Add after the express import
app.use((req, res, next) => {
  res.header('Access-Control-Allow-Origin', '*');
  res.header('Access-Control-Allow-Headers', 'Origin, X-Requested-With, Content-Type, Accept');
  res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  next();
});

// Store current video state
let videoState = {
  currentTime: 0,
  isPlaying: false,
  duration: 0,
  videoPath: ''
};

// Endpoint to get current video state
app.get('/state', (req, res) => {
  res.json(videoState);
});

// Endpoint to update video state
app.post('/sync', (req, res) => {
  videoState = { ...videoState, ...req.body };
  console.log('Video state updated:', videoState);
  res.json({ success: true });
});

// Start the server
app.listen(port, () => {
    console.log(`Server is running on http://localhost:${port}`);
}); 